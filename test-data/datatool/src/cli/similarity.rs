use std::path::PathBuf;

#[derive(Debug, clap::Args)]
pub struct CheckSimilarity {
    samples: Vec<PathBuf>,
}

pub async fn action_check_similarity(args: CheckSimilarity) -> anyhow::Result<()> {
    use crate::model::ExecutableSample;
    use crate::split::NGramIndex;
    use indicatif::ProgressIterator;
    use ndarray::Array;
    use owo_colors::{OwoColorize, Style};
    use std::fs::File;

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
