mod bulk_make_graph;
mod evaluation;
mod similarity;
mod util;

use bulk_make_graph::BulkMakeGraph;
use evaluation::{Evaluate, RunDisasmTool, RunDisasmTools};
use similarity::{CheckSimilarity, SplitSamples};

use crate::fetch;
use crate::model::{CodeVocab, ExecutableSample};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

#[derive(Debug, Parser)]
pub struct Cli {
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
    RunDisasmTools(RunDisasmTools),
    Evaluate(Evaluate),
    CheckSimilarity(CheckSimilarity),
    SplitSamples(SplitSamples),
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

impl Cli {
    pub async fn run(self) -> Result<()> {
        match self.action {
            Action::FetchData(args) => action_sync_data(args).await,
            Action::ShowSample(args) => action_show_sample(args).await,
            Action::SampleToStrippedElf(args) => action_sample_to_stripped_elf(args).await,
            Action::MakeSuperset(args) => action_make_superset(args).await,
            Action::MakeGraph(args) => action_make_graph(args).await,
            Action::BulkMakeGraph(args) => bulk_make_graph::action_bulk_make_graph(args).await,
            Action::PythonCodegen => action_python_codegen().await,
            Action::RunDisasmTool(args) => evaluation::action_run_disasm_tool(args).await,
            Action::RunDisasmTools(args) => evaluation::action_run_disasm_tools(args).await,
            Action::Evaluate(args) => evaluation::action_evaluate(args).await,
            Action::CheckSimilarity(args) => similarity::action_check_similarity(args).await,
            Action::SplitSamples(args) => similarity::action_split_samples(args).await,
        }
    }
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
    let sample = ExecutableSample::deserialize_from(&mut File::open(&args.sample_path)?)?;

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
    let sample = ExecutableSample::deserialize_from(&mut File::open(&args.sample_path)?)?;

    let elf_bytes = sample
        .as_stripped_elf()
        .context("Converting to stripped ELF")?;

    std::fs::write(&args.output_path, elf_bytes).context("Writing stripped ELF")?;

    Ok(())
}

async fn action_make_superset(args: MakeSuperset) -> Result<()> {
    let sample = ExecutableSample::deserialize_from(&mut File::open(&args.sample_path)?)?;
    let superset = sample.into_superset();

    let file = File::create(&args.output_path)?;
    let file = BufWriter::new(file);
    superset.to_parquet(file)?;

    Ok(())
}

async fn action_make_graph(args: MakeGraph) -> Result<()> {
    let sample = ExecutableSample::deserialize_from(&mut File::open(&args.sample_path)?)?;
    let graph = sample.into_graph();

    let vocab = CodeVocab::deserialize_from(File::open(&args.vocab_path)?)?;

    let file = File::create(&args.output_path)?;
    let file = BufWriter::new(file);
    graph.to_npz(&vocab, file)?;

    Ok(())
}

async fn action_python_codegen() -> Result<()> {
    eprintln!("Nothing here");

    Ok(())
}
