use crate::loader::{dump_elf_symbols, load_executable};
use crate::model::interval_set::Interval;
use crate::model::{AddressClasses, ExecutableSample};
use anyhow::{anyhow, Result};
use anyhow::{bail, Context};
use async_stream::try_stream;
use futures_util::Stream;
use object::read::elf::ElfFile32;
use object::read::pe::PeFile32;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::path::Path;

#[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Copy, Clone)]
pub enum ByteWeightPlatform {
    PeX86,
    ElfX86,
}

impl Display for ByteWeightPlatform {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ByteWeightPlatform::PeX86 => write!(f, "pe-x86"),
            ByteWeightPlatform::ElfX86 => write!(f, "elf-x86"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ByteweightSourceInfo {
    pub experiments_path: String,
}

fn read_pe_x86(platform_path: &Path, executable_name: &str) -> Result<ExecutableSample> {
    let executable_path = platform_path.join("binary").join(&executable_name);

    let executable = std::fs::read(&executable_path)
        .with_context(|| format!("Reading executable from {:?}", executable_path))?;
    let executable = PeFile32::parse(executable.as_slice())
        .with_context(|| format!("Parsing PE file from {:?}", executable_path))?;
    let memory = load_executable(&executable)?;

    let functions_path = platform_path.join("gt/function").join(&executable_name);
    let functions = std::fs::read_to_string(&functions_path)
        .with_context(|| format!("Reading functions from {:?}", functions_path))?;
    let functions = functions
        .lines()
        .map(|line| -> Result<_> {
            let parts = line.split(' ').collect::<Vec<_>>();
            let &[start, end] = &parts[..] else {
                bail!("invalid line: {}", line);
            };
            let start = u32::from_str_radix(start, 16)?;
            let end = u32::from_str_radix(end, 16)?;
            Ok((start, end))
        })
        .collect::<Result<Vec<_>>>()
        .context("Parsing functions")?;

    let thunks_path = platform_path.join("gt/thunk").join(&executable_name);
    let thunks = std::fs::read_to_string(&thunks_path)
        .with_context(|| format!("Reading thunks from {:?}", thunks_path))?;
    let thunks = thunks
        .lines()
        .map(|line| u32::from_str_radix(line, 16).context("Parsing thunk"))
        .collect::<Result<Vec<_>>>()?;

    // NOTE: here we don't add true_data (and, well, we don't use it for superset calculation)
    // do we care about it at all?
    let mut classes = AddressClasses::new();
    for (start, end) in functions {
        classes
            .true_instructions
            .push(Interval::from_start_and_end(start, end));
    }
    for thunk in thunks {
        // with thunks we only get the start address, so assume it's one instruction long and disassemble it
        let instr = iced_x86::Decoder::new(
            32,
            memory.execute_all_at(thunk),
            iced_x86::DecoderOptions::NONE,
        )
        .decode();

        classes
            .true_instructions
            .push(Interval::from_start_and_len(thunk, instr.len() as u32));
    }

    ExecutableSample::new(memory, classes).context("Creating sample")
}

fn read_elf_x86(platform_path: &Path, executable_name: &str) -> Result<ExecutableSample> {
    let executable_path = platform_path.join("binary").join(&executable_name);

    let executable = std::fs::read(&executable_path)
        .with_context(|| format!("Reading executable from {:?}", executable_path))?;
    let executable = ElfFile32::parse(executable.as_slice())
        .with_context(|| format!("Parsing ELF file from {:?}", executable_path))?;

    let memory = load_executable(&executable)?;
    let classes = dump_elf_symbols(&memory, &executable)?;

    ExecutableSample::new(memory, classes).context("Creating sample")
}

pub fn fetch_byteweight(
    byteweight: &ByteweightSourceInfo,
) -> impl Stream<Item = Result<(String, ExecutableSample)>> + '_ {
    try_stream! {
        let root_path = Path::new(&byteweight.experiments_path);

        let platforms = vec![
            ("pe-x86", ByteWeightPlatform::PeX86),
            ("elf-x86", ByteWeightPlatform::ElfX86),
        ];

        for (platform_name, platform) in platforms {
            let platform_path = root_path.join(platform_name);
            let executable_names = std::fs::read_dir(platform_path.join("binary"))
                .with_context(|| format!("Reading list of binaries from {:?}", platform_path))?
                .map(|entry| {
                    entry.context("Reading directory").and_then(|entry| {
                        Ok(entry
                            .file_name()
                            .into_string()
                            .map_err(|_| anyhow!("non-utf8 filename"))?)
                    })
                })
                .collect::<Result<Vec<_>>>()?;

            for executable_name in executable_names {
                if executable_name.starts_with("icc_") {
                    // TODO: investigate icc symbols, seem to be noisy, potentially broken
                    continue;
                }
                let sample = match platform {
                    ByteWeightPlatform::PeX86 => read_pe_x86(&platform_path, &executable_name)?,
                    ByteWeightPlatform::ElfX86 => read_elf_x86(&platform_path, &executable_name)?,
                };
                let path = format!("{}/{}", platform_name, executable_name);

                yield (path, sample);
            }
        }
    }
}
