// TODO: write a CLI entrypoint for this
#[allow(unused)]
mod debian;
mod loader;
mod model;

use crate::loader::dump_pdb;
use crate::model::interval_set::Interval;
use crate::model::ExecutableSample;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use object::read::pe::PeFile32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
}

#[derive(Debug, clap::Args)]
struct DumpPdb {
    exe: PathBuf,
    pdb: Option<PathBuf>,
    #[clap(short, long)]
    output: Option<PathBuf>,
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
                distribution: opt.distribution,
                arch: opt.arch,
            }
        }
    }
}

use debian_config::DebianConfigOpt;

#[derive(Debug, clap::Args)]
struct DumpDebian {
    package_names: Vec<String>,
    #[clap(short, long)]
    output_directory: PathBuf,
    #[clap(flatten)]
    config: DebianConfigOpt,
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

    let sample = ExecutableSample::from_pe(&pe_file, &mut pdb)?;

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

async fn main_impl() -> Result<()> {
    let args = Args::parse();

    match args.action {
        Action::DumpPdb(args) => action_dump_pdb(args).await,
        Action::DumpDebian(args) => action_dump_debian(args).await,
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
