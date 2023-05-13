mod cli;
mod disassembly;
mod evaluate;
mod fetch;
mod loader;
mod model;
mod split;

use crate::cli::Cli;
use crate::loader::dump_pdb;
use crate::model::interval_set::Interval;
use clap::Parser;
use tracing::error;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let cli = Cli::parse();

    cli.run().await.unwrap_or_else(|e| {
        error!("Error occurred: {:?}", e);
        std::process::exit(1);
    });
}
