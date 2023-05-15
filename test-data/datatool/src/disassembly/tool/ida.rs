use crate::disassembly::DisassemblyResult;
use crate::model::ExecutableSample;
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use std::collections::BTreeSet;
use tokio::process::Command;
use tracing::debug;

#[derive(Deserialize, Clone)]
pub struct IdaConfig {
    ida_path: String,
    show_output: bool,
}

/// This is just a slightly modified version of analysis.idc from the IDA SDK.
static IDA_SCRIPT: &str = r#"
#include <idc.idc>

static main()
{
  // turn on coagulation of data in the final pass of analysis
  set_inf_attr(INF_AF, get_inf_attr(INF_AF) | AF_DODATA | AF_FINAL);
  // .. and plan the entire address space for the final pass
  auto_mark_range(0, BADADDR, AU_FINAL);

  msg("Waiting for the end of the auto analysis...\n");
  auto_wait();

  msg("\n\n------ Creating the output file.... --------\n");
  auto file = get_idb_path()[0:-4] + ".lst";

  auto fhandle = fopen(file, "w");
  gen_file(OFILE_LST, fhandle, 0, BADADDR, 0); // create the assembler file
  msg("All done, exiting...\n");
  qexit(0); // exit to OS, error code 0 - success
}
"#;

static LST_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\w+:(?P<addr>[0-9A-F]+)(?: (?:[0-9A-F]{2}[ +]+)+(?P<content>.*))?").unwrap()
});

fn parse_lst(lst: &str) -> Result<BTreeSet<u32>> {
    let mut result = BTreeSet::new();

    let mut prev_processed_addr = 0u32;

    for line in lst.lines() {
        if let Some(captures) = LST_REGEX.captures(line) {
            let addr = u32::from_str_radix(&captures["addr"], 16).unwrap();
            if prev_processed_addr == addr {
                continue;
            }

            if let Some(content) = captures.name("content") {
                let content = content.as_str();

                let looks_like_data = content
                    .split(" ")
                    .take(2)
                    .any(|v| matches!(v, "db" | "dw" | "dd" | "dq" | "align"))
                    || content.starts_with(r#"text "UTF-16LE""#);

                prev_processed_addr = addr;

                if !looks_like_data {
                    result.insert(addr);
                }
            }
        }
    }

    Ok(result)
}

pub async fn run_ida(config: &IdaConfig, sample: &ExecutableSample) -> Result<DisassemblyResult> {
    debug!("Running IDA");

    let temp_dir = tempfile::tempdir().context("Failed to create temporary directory")?;

    let elf_path = temp_dir.path().join("sample.elf");
    let script_path = temp_dir.path().join("analysis.idc");

    let elf = sample
        .as_stripped_elf()
        .context("Failed to create stripped ELF")?;
    std::fs::write(&elf_path, elf).context("Failed to write stripped ELF")?;
    std::fs::write(&script_path, IDA_SCRIPT).context("Failed to write IDA script")?;

    let mut command = Command::new(&config.ida_path);

    command
        .arg("-A")
        .arg(format!("-S{}", script_path.to_string_lossy()))
        .arg(&elf_path);

    if !config.show_output {
        command.stdout(std::process::Stdio::null());
        command.stderr(std::process::Stdio::null());
    }

    let exit_status = command
        .spawn()
        .context("Failed to spawn IDA")?
        .wait()
        .await
        .context("Failed to wait for IDA")?;

    if !exit_status.success() {
        anyhow::bail!(
            "IDA exited with non-successful exit code: {:?}",
            exit_status.code()
        );
    }

    let lst_path = temp_dir.path().join("sample.elf.lst");

    let lst = std::fs::read_to_string(&lst_path).context("Failed to read IDA output")?;

    let predicted_instructions = parse_lst(&lst)?;

    Ok(DisassemblyResult {
        predicted_instructions,
    })
}
