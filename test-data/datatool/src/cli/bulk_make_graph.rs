use crate::cli::util::collect_sample_paths;
use crate::model::{CodeVocabBuilder, ExecutableSample};
use anyhow::Context;
use indicatif::ParallelProgressIterator;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::Instant;
use tracing::info;

#[derive(Debug, clap::Args)]
pub struct BulkMakeGraph {
    samples_path: PathBuf,
    #[clap(short, long, default_value_t = 500)]
    vocab_size: usize,
    vocab_out_path: PathBuf,
    graphs_out_path: PathBuf,
}

pub(super) async fn action_bulk_make_graph(args: BulkMakeGraph) -> anyhow::Result<()> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(16)
        .build_global()
        .context("Initializing thread pool")?;

    let samples = collect_sample_paths(&args.samples_path)?;

    info!("Found {} samples", samples.len());

    // let's build the vocab first
    info!("Building vocab...");
    let vocab = samples
        .par_iter()
        .progress_count(samples.len() as u64)
        .map(|sample_path| -> anyhow::Result<_> {
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
        .try_for_each(|sample_path| -> anyhow::Result<()> {
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
