use crate::disassembly::DisassemblyResult;
use crate::model::ExecutableSample;
use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Deserialize;
use shiplift::tty::TtyChunk;
use shiplift::{ContainerOptions, Docker};
use std::collections::BTreeSet;
use std::io::Write;
use tracing::{debug, error, warn};

/// Runs the DeepDi tool in a docker container.
#[derive(Deserialize, Clone)]
pub struct DeepDiConfig {
    pub drm_key: String,
    pub image_name: String,
}

pub async fn run_deepdi(
    config: &DeepDiConfig,
    sample: &ExecutableSample,
) -> Result<DisassemblyResult> {
    debug!("Running DeepDi");

    let docker = Docker::new();

    let sample_elf = sample
        .as_stripped_elf()
        .context("Failed to create stripped ELF")?;

    let mut sample_elf_file =
        tempfile::NamedTempFile::new().context("Failed to create temporary file")?;
    sample_elf_file
        .write_all(&sample_elf)
        .context("Failed to write stripped ELF")?;

    let container = docker
        .containers()
        .create(
            &ContainerOptions::builder(&config.image_name)
                .entrypoint(vec![
                    "/bin/bash".to_string(),
                    "-c".to_string(),
                    format!(
                        "python3 /home/DeepDi.py --key {} --path /mnt/sample.elf",
                        config.drm_key
                    ),
                ])
                .volumes(vec![&format!(
                    "{}:/mnt/sample.elf",
                    sample_elf_file.path().to_str().unwrap()
                )])
                .attach_stdout(true)
                .attach_stderr(true)
                .build(),
        )
        .await
        .context("Failed to create DeepDi container")?;

    let container = docker.containers().get(container.id);

    let tty_multiplexer = container
        .attach()
        .await
        .context("Failed to attach to DeepDi container")?;

    container
        .start()
        .await
        .context("Failed to start DeepDi container")?;

    let (mut reader, _writer) = tty_multiplexer.split();

    let mut output = Vec::new();

    while let Some(tty_result) = reader.next().await {
        match tty_result {
            Ok(TtyChunk::StdOut(stdout)) => output.extend(stdout),
            Ok(TtyChunk::StdErr(stderr)) => {
                warn!("DeepDi stderr: {}", std::str::from_utf8(&stderr).unwrap())
            }
            Ok(TtyChunk::StdIn(_)) => unreachable!(),
            Err(e) => error!("Error reading DeepDi output: {}", e),
        }
    }

    let result = container
        .wait()
        .await
        .context("Failed to wait for DeepDi container")?;

    if result.status_code != 0 {
        anyhow::bail!(
            "DeepDi exited with non-successful exit code: {}. The contained is not deleted.",
            result.status_code
        );
    }

    container
        .delete()
        .await
        .context("Failed to delete DeepDi container")?;

    let output = std::str::from_utf8(&output).context("Failed to parse DeepDi output as string")?;

    let mut predicted_instructions = BTreeSet::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with("0x") {
            continue;
        }

        let address = u32::from_str_radix(&line[2..], 16).context("Failed to parse address")?;
        predicted_instructions.insert(address);
    }

    Ok(DisassemblyResult {
        predicted_instructions,
    })
}
