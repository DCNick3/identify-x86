// TODO: write a CLI entrypoint for this
mod byteweight;
mod debian;
mod disassembly;
mod evaluate;
mod loader;
mod model;

use crate::disassembly::DisasmToolName;
use crate::disassembly::ExecutableDisassembler;
use crate::loader::dump_pdb;
use crate::model::interval_set::Interval;
use crate::model::{CodeVocab, CodeVocabBuilder, ExecutableSample};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use debian_config::DebianConfigOpt;
use indicatif::ParallelProgressIterator;
use object::read::pe::PeFile32;
use rayon::prelude::*;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info};

#[derive(Debug, Parser)]
struct Args {
    #[clap(subcommand)]
    action: Action,
}

#[derive(Debug, Subcommand)]
enum Action {
    DumpPdb(DumpPdb),
    DumpDebian(DumpDebian),
    DumpByteweight(DumpByteweight),
    ShowSample(ShowSample),
    SampleToStrippedElf(SampleToStrippedElf),
    MakeSuperset(MakeSuperset),
    MakeGraph(MakeGraph),
    BulkMakeGraph(BulkMakeGraph),
    PythonCodegen,
    RunDisasmTool(RunDisasmTool),
}

#[allow(unused_parens)]
mod debian_config {
    use crate::debian::DebianConfig;

    #[derive(Debug, clap::Args)]
    pub struct DebianConfigOpt {
        #[clap(long, default_value_t = ("http://deb.debian.org/debian".to_string()))]
        mirror: String,
        #[clap(long, default_value_t = ("http://debug.mirrors.debian.org/debian-debug/".to_string()))]
        debug_mirror: String,
        #[clap(long)]
        no_debug_mirror: bool,
        #[clap(long, default_value_t = ("buster".to_string()))]
        distribution: String,
        #[clap(long)]
        debug_distribution: Option<String>,
        #[clap(long, default_value_t = ("i386".to_string()))]
        arch: String,
    }

    impl From<DebianConfigOpt> for DebianConfig {
        fn from(opt: DebianConfigOpt) -> Self {
            Self {
                mirror: opt.mirror,
                debug_mirror: if opt.no_debug_mirror {
                    None
                } else {
                    Some(opt.debug_mirror)
                },
                debug_distribution: opt
                    .debug_distribution
                    .unwrap_or_else(|| format!("{}-debug", &opt.distribution)),
                distribution: opt.distribution,
                arch: opt.arch,
            }
        }
    }
}

#[derive(Debug, clap::Args)]
struct DumpPdb {
    exe: PathBuf,
    pdb: Option<PathBuf>,
    #[clap(short, long)]
    output: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
struct DumpDebian {
    package_names: Vec<String>,
    #[clap(short, long)]
    output_directory: PathBuf,
    #[clap(flatten)]
    config: DebianConfigOpt,
}

#[derive(Debug, clap::Args)]
struct DumpByteweight {
    experiments_path: PathBuf,
    #[clap(short, long)]
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

fn write_sample(sample: &ExecutableSample, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    info!("Writing sample to {}", path.display());
    let mut file = std::fs::File::create(path)?;
    sample.serialize_into(&mut file)?;
    Ok(())
}

async fn action_dump_pdb(args: DumpPdb) -> Result<()> {
    let pe_data = std::fs::read(&args.exe)?;
    let pe_file = PeFile32::parse(pe_data.as_slice())?;

    let pdb = std::fs::File::open(args.pdb.unwrap_or_else(|| args.exe.with_extension("pdb")))?;
    let mut pdb = pdb::PDB::open(pdb)?;
    let info = pdb.pdb_information()?;

    info!("PDB GUID: {}", info.guid);

    let sample = ExecutableSample::from_pe_and_pdb(&pe_file, &mut pdb)?;

    debug!("{}", sample.classes.dump());

    let (covered, total) = sample.coverage();

    info!(
        "Coverage: {}/{} ({:.2}%)",
        covered,
        total,
        100.0 * covered as f64 / total as f64
    );

    write_sample(
        &sample,
        args.output
            .unwrap_or_else(|| args.exe.with_extension("sample")),
    )?;
    Ok(())
}

async fn save_debian_package(
    package_name: &str,
    path: &str,
    output_directory: &Path,
    sample: ExecutableSample,
) -> Result<()> {
    let path = output_directory
        .join(package_name)
        .join(path.strip_prefix("./").unwrap().replace('/', "_") + ".sample");
    std::fs::create_dir_all(path.parent().unwrap()).context("Creating output directory")?;
    write_sample(&sample, path).context("Writing sample")?;

    Ok(())
}

async fn action_dump_debian(args: DumpDebian) -> Result<()> {
    std::fs::create_dir_all(&args.output_directory).context("Creating output directory")?;

    let output_directory = Arc::new(args.output_directory.clone());
    debian::dump_debian(
        &args.config.into(),
        args.package_names,
        move |package_name, path, sample| {
            let output_directory = output_directory.clone();
            let package_name = package_name.to_string();
            let path = path.to_string();
            async move {
                save_debian_package(&package_name, &path, &output_directory, sample)
                    .await
                    .context("Saving sample")
            }
        },
    )
    .await
    .context("Dumping debian packages")?;

    Ok(())
}

async fn action_dump_byteweight(args: DumpByteweight) -> Result<()> {
    std::fs::create_dir_all(&args.output_directory).context("Creating output directory")?;

    // let output_directory = Arc::new(args.output_directory.clone());
    byteweight::dump_byteweight(&args.experiments_path, |platform, name, sample| {
        let path = args
            .output_directory
            .join(format!("{}", platform))
            .join(name + ".sample");

        std::fs::create_dir_all(path.parent().unwrap()).context("Creating output directory")?;

        write_sample(&sample, path).context("Writing sample")?;
        Ok(())
    })
    .context("Dumping byteweight packages")?;

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
        Action::DumpPdb(args) => action_dump_pdb(args).await,
        Action::DumpDebian(args) => action_dump_debian(args).await,
        Action::DumpByteweight(args) => action_dump_byteweight(args).await,
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
