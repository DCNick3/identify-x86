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
use std::path::PathBuf;

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
}

async fn action_dump_pdb(args: DumpPdb) -> Result<()> {
    let pe_data = std::fs::read(args.exe)?;
    let pe_file = PeFile32::parse(pe_data.as_slice())?;

    let pdb = std::fs::File::open(args.pdb)?;
    let mut pdb = pdb::PDB::open(pdb)?;
    let info = pdb.pdb_information()?;

    println!("PDB GUID: {}", info.guid);

    let sample = ExecutableSample::from_pe(&pe_file, &mut pdb)?;

    println!("{}", sample.classes.dump());

    let (covered, total) = sample.coverage();

    println!(
        "Coverage: {}/{} ({:.2}%)",
        covered,
        total,
        100.0 * covered as f64 / total as f64
    );

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
    main_impl().await.unwrap_or_else(|e| {
        eprintln!("Error occurred: {:?}", e);
        std::process::exit(1);
    });
}
