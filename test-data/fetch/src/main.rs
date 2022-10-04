// TODO: write a CLI entrypoint for this
#[allow(unused)]
mod debian;
mod loader;
mod model;

use crate::loader::dump_pdb;
use crate::model::interval_set::Interval;
use crate::model::ExecutableSample;
use anyhow::Result;
use clap::{Parser, Subcommand};
use object::read::pe::PeFile32;
use std::path::{Path, PathBuf};
use tracing::{debug, error, info};

#[derive(Debug, Parser)]
struct Args {
    #[clap(subcommand)]
    action: Action,
}

#[derive(Debug, Subcommand)]
enum Action {
    DumpPdb(DumpPdb),
}

#[derive(Debug, clap::Args)]
struct DumpPdb {
    exe: PathBuf,
    pdb: PathBuf,
    #[clap(short, long)]
    output: PathBuf,
}

fn write_sample(sample: &ExecutableSample, path: &Path) -> Result<()> {
    info!("Writing sample to {}", path.display());
    let mut file = std::fs::File::create(path)?;
    sample.serialize_into(&mut file)?;
    Ok(())
}

async fn action_dump_pdb(args: DumpPdb) -> Result<()> {
    let pe_data = std::fs::read(args.exe)?;
    let pe_file = PeFile32::parse(pe_data.as_slice())?;

    let pdb = std::fs::File::open(args.pdb)?;
    let mut pdb = pdb::PDB::open(pdb)?;
    let info = pdb.pdb_information()?;

    info!("PDB GUID: {}", info.guid);

    let sample = ExecutableSample::from_pe(&pe_file, &mut pdb)?;

    debug!("{}", sample.classes.dump());

    let (covered, total) = sample.coverage();

    info!(
        "Coverage: {}/{} ({:.2}%)",
        covered,
        total,
        100.0 * covered as f64 / total as f64
    );

    write_sample(&sample, &args.output)?;
    Ok(())
}

async fn main_impl() -> Result<()> {
    let args = Args::parse();

    match args.action {
        Action::DumpPdb(args) => action_dump_pdb(args).await,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt::init();

    main_impl().await.unwrap_or_else(|e| {
        error!("Error occurred: {:?}", e);
        std::process::exit(1);
    });
}
