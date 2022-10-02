mod debian;
mod loader;
mod model;

use crate::loader::dump_pdb;
use crate::model::ExecutableSample;
use anyhow::Result;

async fn save_sample(sample: ExecutableSample) -> Result<()> {
    todo!()
}

async fn main_impl() -> Result<()> {
    let file = std::fs::File::open("test-data/llvm/RelWithDebInfo/llvm-tblgen.pdb")?;
    let mut pdb = pdb::PDB::open(file)?;
    let classes = dump_pdb(0x400000, &mut pdb)?;

    println!("{}", classes.dump());

    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    main_impl().await.unwrap_or_else(|e| {
        eprintln!("Error occurred: {:?}", e);
        std::process::exit(1);
    });
}
