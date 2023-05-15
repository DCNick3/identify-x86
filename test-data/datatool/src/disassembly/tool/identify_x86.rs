use crate::disassembly::DisassemblyResult;
use crate::model::{CodeVocab, ExecutableSample};
use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Deserialize;
use shiplift::tty::TtyChunk;
use shiplift::{ContainerOptions, Docker};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::Write;
use std::str::FromStr;
use tracing::{debug, error};

/// Runs the identify-x86 tool in a docker container.
#[derive(Deserialize, Clone)]
pub struct IdentifyX86Config {
    pub model_path: String,
    pub code_vocab_path: String,
    pub image_name: String,
}

pub async fn run_identify_x86(
    config: &IdentifyX86Config,
    sample: &ExecutableSample,
) -> Result<DisassemblyResult> {
    debug!("Running IdentifyX86");

    let docker = Docker::new();

    debug!("Computing the graph");
    let sample_superset = sample.clone().into_superset();
    let sample_graph = sample_superset.clone().into_graph();

    let code_vocab = CodeVocab::deserialize_from(
        File::open(&config.code_vocab_path)
            .with_context(|| format!("Opening code vocab file {}", config.code_vocab_path))?,
    )?;

    let mut sample_graph_file =
        tempfile::NamedTempFile::new().context("Failed to create temporary file")?;
    sample_graph
        .to_npz(&code_vocab, &mut sample_graph_file)
        .context("Failed to write graph")?;

    sample_graph_file
        .flush()
        .context("Failed to flush graph file")?;

    debug!("Running identify-x86 model in docker");

    let container = docker
        .containers()
        .create(
            &ContainerOptions::builder(&config.image_name)
                .volumes(vec![
                    &format!("{}:/model.pt", config.model_path),
                    &format!(
                        "{}:/sample.graph",
                        sample_graph_file.path().to_str().unwrap()
                    ),
                ])
                .cmd(vec!["/model.pt", "/sample.graph"])
                .attach_stdout(true)
                .attach_stderr(true)
                .build(),
        )
        .await
        .context("Failed to create identify-x86 container")?;

    let container = docker.containers().get(container.id);

    let tty_multiplexer = container
        .attach()
        .await
        .context("Failed to attach to identify-x86 container")?;

    container
        .start()
        .await
        .context("Failed to start identify-x86 container")?;

    let (mut reader, _writer) = tty_multiplexer.split();

    let mut output = Vec::new();

    while let Some(tty_result) = reader.next().await {
        match tty_result {
            Ok(TtyChunk::StdOut(stdout)) => output.extend(stdout),
            Ok(TtyChunk::StdErr(stderr)) => {
                debug!(
                    "IdentifyX86 stderr: {}",
                    std::str::from_utf8(&stderr).unwrap()
                )
            }
            Ok(TtyChunk::StdIn(_)) => unreachable!(),
            Err(e) => error!("Error reading identify-x86 output: {}", e),
        }
    }

    let result = container
        .wait()
        .await
        .context("Failed to wait for identify-x86 container")?;

    drop(sample_graph_file);

    if result.status_code != 0 {
        anyhow::bail!(
            "identify-x86 exited with non-successful exit code: {}. The contained is not deleted.",
            result.status_code
        );
    }

    container
        .delete()
        .await
        .context("Failed to delete identify-x86 container")?;

    let output =
        std::str::from_utf8(&output).context("Failed to parse identify-x86 output as string")?;

    let mut predicted_nodes = BTreeSet::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let address = u32::from_str(line).context("Failed to parse node index")?;
        predicted_nodes.insert(address);
    }

    let predicted_instructions = predicted_nodes
        .into_iter()
        .map(|v| sample_superset.superset[v as usize].0)
        .collect();

    Ok(DisassemblyResult {
        predicted_instructions,
    })
}
