use crate::cli::util::collect_sample_paths;
use crate::disassembly::{DisasmToolConfig, DisasmToolName, ExecutableDisassembler};
use crate::evaluate;
use crate::model::ExecutableSample;
use anyhow::Context;
use indicatif::ProgressIterator;
use prettytable::{row, Table};
use serde::Serialize;
use std::fs::File;
use std::path::PathBuf;
use std::time::Instant;
use strum::IntoEnumIterator;
use tracing::{error, info, info_span, Instrument};

#[derive(Debug, clap::Args)]
pub struct RunDisasmTool {
    tool: DisasmToolName,
    sample_path: PathBuf,
    #[clap(short, long)]
    output_path: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
pub struct RunDisasmTools {
    sample_path: PathBuf,
    #[clap(short, long)]
    output_path: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
pub struct Evaluate {
    samples_path: PathBuf,
    csv_path: PathBuf,
}

fn load_runner_config() -> anyhow::Result<DisasmToolConfig> {
    let config_file = "runners.yaml";
    let config = std::fs::read_to_string("runners.yaml")
        .with_context(|| format!("Reading runners config file {}", config_file))?;
    let config = serde_yaml::from_str(&config)
        .with_context(|| format!("Parsing runners config file {} as YAML", config_file))?;
    Ok(config)
}

pub async fn action_run_disasm_tool(args: RunDisasmTool) -> anyhow::Result<()> {
    let sample = ExecutableSample::deserialize_from(&mut File::open(&args.sample_path)?)?;

    let config = load_runner_config()?;

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

pub async fn action_run_disasm_tools(args: RunDisasmTools) -> anyhow::Result<()> {
    let sample = ExecutableSample::deserialize_from(&mut File::open(&args.sample_path)?)?;
    let superset = sample.clone().into_superset();

    let config = load_runner_config()?;

    let mut table = Table::new();
    table.set_format(*prettytable::format::consts::FORMAT_BORDERS_ONLY);
    table.set_titles(row![
        "Tool",
        "True Positives",
        "False Positives",
        "False Negatives",
        "Precision",
        "Recall",
        "F1",
        "Time",
    ]);

    for tool in DisasmToolName::iter().progress() {
        let start = Instant::now();
        let result = match tool
            .with_config(&config)
            .disassemble(&sample)
            .await
            .context("Running disasm tool")
        {
            Ok(v) => v,
            Err(e) => {
                error!("Error running {:?}: {}", tool, e);
                continue;
            }
        };

        // Note that this time includes docker overhead!
        let time = start.elapsed();

        let eval = evaluate::evaluate_result(&superset, &result);
        let s = eval.summary();

        table.add_row(row![
            format!("{:?}", tool),
            s.true_positives,
            s.false_positives,
            s.false_negatives,
            format!("{:.05}", s.precision),
            format!("{:.05}", s.recall),
            format!("{:.05}", s.f1),
            format!("{:.02}s", time.as_secs_f64())
        ]);
    }

    println!("{}", table);

    Ok(())
}

pub async fn action_evaluate(args: Evaluate) -> anyhow::Result<()> {
    let samples = collect_sample_paths(&args.samples_path)?;
    info!("Found {} samples", samples.len());

    let config = load_runner_config()?;

    #[derive(Serialize)]
    struct CsvRecord {
        tool: DisasmToolName,
        sample: String,
        size: u64,
        true_positives: usize,
        false_positives: usize,
        false_negatives: usize,
        precision: f64,
        recall: f64,
        f1: f64,
        time: f64,
    }

    let mut csv = csv::Writer::from_path(&args.csv_path).context("Creating output CSV")?;

    let path_prefix = args.samples_path.to_str().unwrap();

    for (i, sample_path) in samples.iter().enumerate() {
        let sample_name = sample_path
            .to_str()
            .unwrap()
            .strip_prefix(path_prefix)
            .unwrap()
            .strip_prefix("/")
            .unwrap()
            .strip_suffix(".sample")
            .unwrap();

        let sample = ExecutableSample::deserialize_from(&mut File::open(&sample_path)?)?;
        let superset = sample.clone().into_superset();

        let sample_size = sample.size();

        for tool in DisasmToolName::iter() {
            info!(
                "[{}/{} {:.01}%] Running {:?} on [{}] {}",
                i + 1,
                samples.len(),
                i as f64 / samples.len() as f64 * 100.0,
                tool,
                sample_size,
                sample_name
            );

            let start = Instant::now();
            let result = match tool
                .with_config(&config)
                .disassemble(&sample)
                .instrument(info_span!("disassemble", tool = ?tool, sample_name))
                .await
                .context("Running disasm tool")
            {
                Ok(v) => v,
                Err(e) => {
                    error!("Error running {:?}: {}", tool, e);
                    continue;
                }
            };
            // Note that this time includes docker overhead!
            let time = start.elapsed();

            let eval = evaluate::evaluate_result(&superset, &result);
            let s = eval.summary();

            let record = CsvRecord {
                tool,
                sample: sample_name.to_string(),
                size: sample_size,
                true_positives: s.true_positives,
                false_positives: s.false_positives,
                false_negatives: s.false_negatives,
                precision: s.precision,
                recall: s.recall,
                f1: s.f1,
                time: time.as_secs_f64(),
            };

            csv.serialize(record)?;
            csv.flush()?;
        }
    }

    Ok(())
}
