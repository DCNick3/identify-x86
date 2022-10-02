mod debian;
mod loader;
mod model;

use crate::loader::dump_pdb;
use crate::model::interval_set::Interval;
use crate::model::ExecutableSample;
use anyhow::Result;
use object::read::pe::PeFile32;

async fn save_sample(sample: ExecutableSample) -> Result<()> {
    todo!()
}

async fn main_impl() -> Result<()> {
    let pe_data = std::fs::read("test-data/llvm/RelWithDebInfo/llvm-tblgen.exe")?;
    let pe_file = PeFile32::parse(pe_data.as_slice())?;

    let pdb = std::fs::File::open("test-data/llvm/RelWithDebInfo/llvm-tblgen.pdb")?;
    let mut pdb = pdb::PDB::open(pdb)?;

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

#[tokio::main(flavor = "current_thread")]
async fn main() {
    main_impl().await.unwrap_or_else(|e| {
        eprintln!("Error occurred: {:?}", e);
        std::process::exit(1);
    });
}
