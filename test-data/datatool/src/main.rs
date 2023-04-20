mod disassembly;
mod evaluate;
mod fetch;
mod loader;
mod model;

use crate::disassembly::DisasmToolName;
use crate::disassembly::ExecutableDisassembler;
use crate::loader::dump_pdb;
use crate::model::interval_set::Interval;
use crate::model::{CodeVocab, CodeVocabBuilder, ExecutableSample};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use indicatif::ParallelProgressIterator;
use rayon::prelude::*;
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::Instant;
use tracing::{error, info};

#[derive(Debug, Parser)]
struct Args {
    #[clap(subcommand)]
    action: Action,
}

#[derive(Debug, Subcommand)]
enum Action {
    FetchData(SyncData),
    ShowSample(ShowSample),
    SampleToStrippedElf(SampleToStrippedElf),
    MakeSuperset(MakeSuperset),
    MakeGraph(MakeGraph),
    BulkMakeGraph(BulkMakeGraph),
    PythonCodegen,
    RunDisasmTool(RunDisasmTool),
}

#[derive(Debug, clap::Args)]
struct SyncData {
    #[clap(long, default_value = "sources.yaml")]
    sources_config: PathBuf,
    #[clap(long, default_value = "test-data/samples")]
    output_directory: PathBuf,
}

#[derive(Debug, clap::Args)]
struct ShowSample {
    sample_path: PathBuf,
    #[clap(short, long)]
    dump_ranges: bool,
}

#[derive(Debug, clap::Args)]
struct SampleToStrippedElf {
    sample_path: PathBuf,
    output_path: PathBuf,
}

#[derive(Debug, clap::Args)]
struct MakeSuperset {
    sample_path: PathBuf,
    output_path: PathBuf,
}

#[derive(Debug, clap::Args)]
struct MakeGraph {
    sample_path: PathBuf,
    vocab_path: PathBuf,
    output_path: PathBuf,
}

#[derive(Debug, clap::Args)]
struct BulkMakeGraph {
    samples_path: PathBuf,
    #[clap(short, long, default_value_t = 500)]
    vocab_size: usize,
    vocab_out_path: PathBuf,
    graphs_out_path: PathBuf,
}

#[derive(Debug, clap::Args)]
struct RunDisasmTool {
    tool: DisasmToolName,
    sample_path: PathBuf,
    #[clap(short, long)]
    output_path: Option<PathBuf>,
}

async fn action_sync_data(args: SyncData) -> Result<()> {
    let config_path = args.sources_config;
    let config = std::fs::read_to_string(&config_path)
        .with_context(|| format!("Reading sources config file {}", config_path.display()))?;
    let config = serde_yaml::from_str(&config)
        .with_context(|| format!("Parsing sources config file {}", config_path.display()))?;

    fetch::sync_sources_to_directory(&config, &args.output_directory)
        .await
        .context("Fetching sources")?;

    Ok(())
}

async fn action_show_sample(args: ShowSample) -> Result<()> {
    let sample = ExecutableSample::deserialize_from(&mut std::fs::File::open(&args.sample_path)?)?;

    println!("Memory map:");
    println!("{}", sample.memory.map());

    if args.dump_ranges {
        println!("Ranges:");
        println!("{}", sample.classes.dump());
    }

    let coverage = sample.coverage();
    let coverage_float = sample.coverage_float();
    println!(
        "Coverage: {}/{} ({:.2}%)",
        coverage.0,
        coverage.1,
        coverage_float * 100.0
    );

    Ok(())
}

async fn action_sample_to_stripped_elf(args: SampleToStrippedElf) -> Result<()> {
    let sample = ExecutableSample::deserialize_from(&mut std::fs::File::open(&args.sample_path)?)?;

    let elf_bytes = sample
        .as_stripped_elf()
        .context("Converting to stripped ELF")?;

    std::fs::write(&args.output_path, elf_bytes).context("Writing stripped ELF")?;

    Ok(())
}

async fn action_make_superset(args: MakeSuperset) -> Result<()> {
    let sample = ExecutableSample::deserialize_from(&mut std::fs::File::open(&args.sample_path)?)?;
    let superset = sample.into_superset();

    let file = std::fs::File::create(&args.output_path)?;
    let file = BufWriter::new(file);
    superset.to_parquet(file)?;

    Ok(())
}

async fn action_make_graph(args: MakeGraph) -> Result<()> {
    let sample = ExecutableSample::deserialize_from(&mut std::fs::File::open(&args.sample_path)?)?;
    let graph = sample.into_graph();

    let vocab = CodeVocab::deserialize_from(std::fs::File::open(&args.vocab_path)?)?;

    let file = std::fs::File::create(&args.output_path)?;
    let file = BufWriter::new(file);
    graph.to_npz(&vocab, file)?;

    Ok(())
}

async fn action_bulk_make_graph(args: BulkMakeGraph) -> Result<()> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(16)
        .build_global()
        .context("Initializing thread pool")?;

    let samples = walkdir::WalkDir::new(&args.samples_path)
        .into_iter()
        .filter(|e| {
            e.as_ref()
                .map(|e| e.path().extension().unwrap_or_default() == "sample")
                .unwrap_or(false)
        })
        .map(|r| r.map(|e| e.into_path()).map_err(|e| e.into()))
        .collect::<Result<Vec<PathBuf>>>()?;

    info!("Found {} samples", samples.len());

    // let's build the vocab first
    info!("Building vocab...");
    let vocab = samples
        .par_iter()
        .progress_count(samples.len() as u64)
        .map(|sample_path| -> Result<_> {
            // info!("Processing {}", sample_path.display());
            let mut b = CodeVocabBuilder::new();
            let sample =
                ExecutableSample::deserialize_from(&mut std::fs::File::open(&sample_path)?)?;
            let superset = sample.into_superset();
            b.add_sample(&superset);
            Ok(b)
        })
        .try_reduce(
            || CodeVocabBuilder::new(),
            |mut a, b| {
                a.merge(b);
                Ok(a)
            },
        )?
        .build_top_k(args.vocab_size);

    // now to the real work!
    info!("Building graphs...");
    samples
        .par_iter()
        // .progress_count(samples.len() as u64)
        .try_for_each(|sample_path| -> Result<()> {
            let start = Instant::now();
            let sample =
                ExecutableSample::deserialize_from(&mut std::fs::File::open(&sample_path)?)?;
            let superset_sample = sample.into_superset();
            info!(
                "{:>150}: {:07} nodes",
                sample_path.display(),
                superset_sample.superset.len(),
            );
            let node_count = superset_sample.superset.len();

            if node_count > 5000000 {
                info!(
                    "{:>150}: too much nodes, skipping, it will explode later down the line",
                    sample_path.display()
                );
                return Ok(());
            }

            let graph = superset_sample.into_graph();
            let edges_count = graph.graph.edges.len();

            let output_path = args
                .graphs_out_path
                .join(sample_path.strip_prefix(&args.samples_path).unwrap())
                .with_extension("graph");

            std::fs::create_dir_all(output_path.parent().unwrap())?;

            let file = std::fs::File::create(&output_path)?;
            let file = BufWriter::new(file);
            graph.to_npz(&vocab, file)?;

            let time = start.elapsed();

            info!(
                "{:>150}: {:07} nodes {:09} edges in {:03.04}s",
                sample_path.display(),
                node_count,
                edges_count,
                time.as_secs_f64(),
            );

            Ok(())
        })?;

    // don't forget the vocab!
    vocab.serialize_to(std::fs::File::create(args.vocab_out_path)?)?;
    vocab.serialize_to(std::fs::File::create(
        args.graphs_out_path.join("code.vocab"),
    )?)?;

    Ok(())
}

async fn action_python_codegen() -> Result<()> {
    // let mut tracer = serde_reflection::Tracer::new(serde_reflection::TracerConfig::default());
    //
    // tracer
    //     .trace_simple_type::<SupersetSample>()
    //     .map_err(|e| anyhow!("Tracing superset sample: {}", e))?;
    // let registry = tracer
    //     .registry()
    //     .map_err(|e| anyhow!("Tracing registry: {}", e))?;
    //
    // let mut source = Vec::new();
    // let config = serde_generate::CodeGeneratorConfig::new("identify_x86_data".to_string())
    //     .with_encodings(vec![serde_generate::Encoding::Bincode]);
    // let generator = serde_generate::python3::CodeGenerator::new(&config);
    // generator.output(&mut source, &registry)?;
    //
    // println!("{}", String::from_utf8_lossy(&source));

    Ok(())
}

async fn action_run_disasm_tool(args: RunDisasmTool) -> Result<()> {
    let sample = ExecutableSample::deserialize_from(&mut std::fs::File::open(&args.sample_path)?)?;

    let config_file = "runners.yaml";
    let config = std::fs::read_to_string("runners.yaml")
        .with_context(|| format!("Reading runners config file {}", config_file))?;
    let config = serde_yaml::from_str(&config)
        .with_context(|| format!("Parsing runners config file {} as YAML", config_file))?;

    let result = args
        .tool
        .with_config(&config)
        .disassemble(&sample)
        .await
        .context("Running disasm tool")?;

    let mut output: Box<dyn std::io::Write> = match args.output_path {
        Some(v) => Box::new(File::create(v)?),
        None => Box::new(std::io::sink()),
    };

    for &instr_addr in &result.predicted_instructions {
        writeln!(output, "0x{:x}", instr_addr).context("Writing to output")?;
    }

    let superset = sample.into_superset();

    let eval = evaluate::evaluate_result(&superset, &result);
    let eval_summary = eval.summary();

    println!("{:#?}", eval_summary);

    Ok(())
}

async fn main_impl() -> Result<()> {
    let args = Args::parse();

    match args.action {
        Action::FetchData(args) => action_sync_data(args).await,
        Action::ShowSample(args) => action_show_sample(args).await,
        Action::SampleToStrippedElf(args) => action_sample_to_stripped_elf(args).await,
        Action::MakeSuperset(args) => action_make_superset(args).await,
        Action::MakeGraph(args) => action_make_graph(args).await,
        Action::BulkMakeGraph(args) => action_bulk_make_graph(args).await,
        Action::PythonCodegen => action_python_codegen().await,
        Action::RunDisasmTool(args) => action_run_disasm_tool(args).await,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    main_impl().await.unwrap_or_else(|e| {
        error!("Error occurred: {:?}", e);
        std::process::exit(1);
    });
}
