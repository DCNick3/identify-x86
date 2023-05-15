use crate::cli::util::collect_sample_paths;
use crate::model::ExecutableSample;
use crate::split::{NGramIndex, SplitBuilder};
use indicatif::{ParallelProgressIterator, ProgressIterator};
use itertools::Itertools;
use ndarray::Array;
use owo_colors::{OwoColorize, Style};
use rayon::prelude::*;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use tracing::info;

#[derive(Debug, clap::Args)]
pub struct CheckSimilarity {
    samples: Vec<PathBuf>,
}

#[derive(Debug, clap::Args)]
pub struct SplitSamples {
    samples_path: PathBuf,
    #[clap(short, long, default_value_t = 0.2)]
    test_proportion: f64,
    labels_out_path: PathBuf,
}

pub async fn action_check_similarity(args: CheckSimilarity) -> anyhow::Result<()> {
    const N: usize = 4;

    let samples = args
        .samples
        .iter()
        .map(|path| {
            File::open(path)
                .map_err(anyhow::Error::from)
                .and_then(|mut f| ExecutableSample::deserialize_from(&mut f))
                .map(|s| NGramIndex::<N>::new(&s.memory))
        })
        .progress()
        .collect::<anyhow::Result<Vec<_>>>()?;

    // print index sizes
    for (i, (s, path)) in samples.iter().zip(args.samples.iter()).enumerate() {
        let sample_name = path.file_name().unwrap().to_str().unwrap();
        println!("{} {:>50}: {}", i, sample_name, s.len());
    }

    let mut sim_matx = Array::zeros((samples.len(), samples.len()));

    // the similarity matrix is symmetric, so we only need to compute the upper triangle
    for i in 0..samples.len() {
        for j in i..samples.len() {
            let sim = samples[i].similarity(&samples[j]);
            sim_matx[[i, j]] = sim;
            sim_matx[[j, i]] = sim;
        }
    }

    for (i, path) in (0..samples.len()).zip(args.samples.iter()) {
        let sample_name = path.file_name().unwrap().to_str().unwrap();
        print!("{:>50}:", sample_name);
        for j in 0..samples.len() {
            let s = if i == j {
                Style::new().bold()
            } else {
                Style::new()
            };
            print!(" {:>0.2}", sim_matx[[i, j]].style(s));
        }
        println!();
    }

    Ok(())
}

pub async fn action_split_samples(args: SplitSamples) -> anyhow::Result<()> {
    use petgraph::prelude::*;

    let sample_paths = collect_sample_paths(&args.samples_path)?;

    info!("Found {} samples", sample_paths.len());

    const NGRAMS_N: usize = 4;

    info!("Loading samples...");
    let samples = sample_paths
        .par_iter()
        .progress()
        .map(|p| {
            File::open(p)
                .map_err(anyhow::Error::from)
                .and_then(|mut f| ExecutableSample::deserialize_from(&mut f))
                .map(|sample| {
                    let size = sample.size();
                    let ngrams = NGramIndex::<NGRAMS_N>::new(&sample.memory);

                    (size, ngrams)
                })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let sample_sizes = samples.iter().map(|(size, _)| *size).collect::<Vec<_>>();
    let samples_ngrams = samples
        .into_iter()
        .map(|(_, ngrams)| ngrams)
        .collect::<Vec<_>>();

    info!("Computing similarity matrix...");

    let similarity_matrix = Array::zeros((samples_ngrams.len(), samples_ngrams.len()));
    let similarity_matrix = Mutex::new(similarity_matrix);

    (0..samples_ngrams.len())
        .into_par_iter()
        .progress()
        .for_each(|i| {
            for j in i..samples_ngrams.len() {
                let sim = samples_ngrams[i].similarity(&samples_ngrams[j]);

                let mut similarity_matrix = similarity_matrix.lock().unwrap();
                similarity_matrix[[i, j]] = sim;
                similarity_matrix[[j, i]] = sim;
            }
        });
    drop(samples_ngrams);

    let similarity_matrix = similarity_matrix.into_inner().unwrap();

    const SIMILARITY_THRESHOLD: f64 = 0.3;

    info!("Computing connected components...");

    // now, find all connected components in the graph defined by the `similarity_matrix[[i, j]] > SIMILARITY_THRESHOLD`
    let mut graph = Graph::<(), (), Undirected>::default();
    for _ in 0..similarity_matrix.nrows() {
        graph.add_node(());
    }
    for i in 0..similarity_matrix.nrows() {
        for j in i..similarity_matrix.ncols() {
            if similarity_matrix[[i, j]] > SIMILARITY_THRESHOLD {
                graph.add_edge(NodeIndex::new(i), NodeIndex::new(j), ());
            }
        }
    }

    let scc = petgraph::algo::tarjan_scc(&graph);
    let scc = scc
        .into_iter()
        .map(|scc| {
            let size = scc.iter().map(|i| sample_sizes[i.index()]).sum::<u64>();
            (scc, size)
        })
        .sorted_by_key(|v| -(v.1 as i64))
        .collect::<Vec<_>>();

    info!("Found {} connected components", scc.len());

    info!("Computing the split...");
    let mut split_builder = SplitBuilder::new(&[1.0 - args.test_proportion, args.test_proportion]);
    for (group, group_size) in scc {
        split_builder.push_component(group.iter().map(|i| i.index()), group_size);
    }

    let split = split_builder.build();
    let [train, test] = split.as_slice() else { unreachable!() };

    info!(
        "Split target: {:0.3}/{:0.3}, actual: {:0.3}/{:0.3}",
        train.target_fraction, test.target_fraction, train.actual_fraction, test.actual_fraction
    );

    let path_prefix = args.samples_path.to_str().unwrap();

    let mut output = File::create(&args.labels_out_path)?;
    for (group, group_name) in split.iter().zip(&["train", "test"]) {
        for sample_path in group.items.iter().map(|&i| &sample_paths[i]).sorted() {
            let sample_name = sample_path
                .to_str()
                .unwrap()
                .strip_prefix(path_prefix)
                .unwrap()
                .strip_prefix("/")
                .unwrap()
                .strip_suffix(".sample")
                .unwrap();

            writeln!(output, "{} {}", group_name, sample_name)?;
        }
    }

    Ok(())
}
