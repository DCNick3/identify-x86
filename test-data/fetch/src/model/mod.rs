pub mod interval_set;

use crate::loader::dump_elf_symbols;
use crate::loader::load_executable;
use crate::{dump_pdb, Interval};
use anyhow::{anyhow, bail, Result};
use interval_set::IntervalSet;
use itertools::Itertools;
use memory_image::MemoryImage;
use object::read::elf::ElfFile32;
use object::read::pe::PeFile32;
use pdb::PDB;
use serde::{Deserialize, Serialize};
use std::io::Write;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone)]
pub struct AddressClasses {
    pub true_instructions: IntervalSet<u32>,
    pub true_data: IntervalSet<u32>,
}

impl AddressClasses {
    pub fn new() -> Self {
        Self {
            true_instructions: IntervalSet::new(),
            true_data: IntervalSet::new(),
        }
    }
    pub fn relocate(&mut self, offset: u32) {
        self.true_instructions.shift(offset);
        self.true_data.shift(offset);
    }

    pub fn filter_to(&mut self, range: Interval<u32>) {
        let mut true_instructions = IntervalSet::new();
        true_instructions.extend(
            self.true_instructions
                .iter()
                .map(|v| v.intersection(range))
                .filter(|v| !v.is_empty()),
        );
        let mut true_data = IntervalSet::new();
        true_data.extend(
            self.true_data
                .iter()
                .map(|v| v.intersection(range))
                .filter(|v| !v.is_empty()),
        );

        self.true_instructions = true_instructions;
        self.true_data = true_data;
    }

    pub fn coverage(&self) -> u32 {
        // TODO: handle interval overlaps
        self.true_instructions
            .iter()
            .map(|v| v.len())
            .chain(self.true_data.iter().map(|v| v.len()))
            .sum()
    }

    pub fn dump(&self) -> String {
        let mut data = self
            .true_instructions
            .iter()
            .map(|i| (i, "code"))
            .chain(self.true_data.iter().map(|i| (i, "data")))
            .collect::<Vec<_>>();
        data.sort();

        let mut result = String::new();
        let mut prev = None;
        for (interval, kind) in data {
            use std::fmt::Write;

            if let Some(prev) = prev {
                let gap = interval.start() - prev;
                if gap > 0 {
                    writeln!(
                        result,
                        "0x{:08x} - 0x{:08x} (0x{:04x}) gap",
                        prev,
                        interval.start(),
                        gap
                    )
                    .unwrap();
                }
            }

            writeln!(
                result,
                "0x{:08x} - 0x{:08x} (0x{:04x}) {}",
                interval.start(),
                interval.end(),
                interval.len(),
                kind
            )
            .unwrap();
            prev = Some(interval.end());
        }

        result
    }
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone)]
pub enum SampleSource {
    Pdb {
        uuid: Uuid,
        path: String,
    },
    Debian {
        package_name: String,
        path: String,
        build_id: Vec<u8>,
    },
}

#[derive(Serialize, Deserialize)]
pub struct ExecutableSample {
    pub memory: MemoryImage,
    pub classes: AddressClasses,
    pub source: Option<SampleSource>,
}

impl ExecutableSample {
    fn from_helper(
        memory: MemoryImage,
        mut classes: AddressClasses,
        source: SampleSource,
    ) -> Result<Self> {
        let exe_item = memory
            .iter()
            .filter(|v| v.protection.contains(memory_image::Protection::EXECUTE))
            .exactly_one()
            .map_err(|_| anyhow::anyhow!("Either no executable section or more than one"))?;

        let exe_range = Interval::from_start_and_end(exe_item.addr, exe_item.end());

        classes.filter_to(exe_range);

        // preserve only the executable section
        let memory = MemoryImage::from_iter(vec![exe_item].into_iter());

        Ok(ExecutableSample {
            memory,
            classes,
            source: Some(source),
        })
    }

    pub fn from_debian(
        package_name: &str,
        path: &str,
        executable: &ElfFile32,
        debug_info: Option<&ElfFile32>,
    ) -> Result<Self> {
        use object::{Object, ObjectSection};

        let build_id = Vec::from(
            executable
                .build_id()?
                .ok_or_else(|| anyhow::anyhow!("no build id"))?,
        );

        let source = SampleSource::Debian {
            package_name: package_name.to_string(),
            path: path.to_string(),
            build_id,
        };

        let memory = load_executable(executable)?;
        let mut classes = dump_elf_symbols(&memory, debug_info.unwrap_or(executable))?;

        // we expect a "normal" gcc-produced binary, so we can expect that all the interesting code will be in .text
        let text = executable
            .section_by_name(".text")
            .ok_or_else(|| anyhow!("no .text section"))?;

        // construct memory image only from the .text section
        let text_memory =
            MemoryImage::from_code_region(text.address() as u32, text.data().unwrap());

        classes.filter_to(Interval::from_start_and_len(
            text.address().try_into().unwrap(),
            text.size().try_into().unwrap(),
        ));

        Self::from_helper(text_memory, classes, source)
    }

    pub fn from_pe<'s, S: std::io::Read + std::io::Seek + std::fmt::Debug + 's>(
        executable: &PeFile32,
        debug_info: &mut PDB<'s, S>,
    ) -> Result<Self> {
        use object::Object;

        let source = if let Some(pdb_info) = executable.pdb_info()? {
            let provided_guid = debug_info.pdb_information()?.guid;
            let expected_guid = uuid::Uuid::from_slice_le(&pdb_info.guid())?;
            if provided_guid != expected_guid {
                bail!(
                    "PDB GUID mismatch: expected {:?}, got {:?}",
                    expected_guid,
                    provided_guid
                );
            }

            SampleSource::Pdb {
                path: String::from_utf8_lossy(pdb_info.path()).to_string(),
                uuid: expected_guid,
            }
        } else {
            bail!("PE file does not contain PDB info");
        };

        let memory = load_executable(executable)?;
        let classes = dump_pdb(
            executable.relative_address_base().try_into().unwrap(),
            debug_info,
        )?;

        Self::from_helper(memory, classes, source)
    }

    pub fn coverage(&self) -> (u32, u32) {
        let covered = self.classes.coverage();
        let total = self
            .memory
            .iter()
            .map(|v| v.data.len())
            .sum::<usize>()
            .try_into()
            .unwrap();
        (covered, total)
    }

    pub fn coverage_float(&self) -> f64 {
        let (covered, total) = self.coverage();
        covered as f64 / total as f64
    }

    pub fn serialize_into(&self, output: &mut impl Write) -> Result<()> {
        let mut output = zstd::stream::write::Encoder::new(
            output, 6, /* tuned to be not too big (file), not too slow (compression) */
        )?;
        bincode::serialize_into(&mut output, self)?;
        output.finish()?;
        Ok(())
    }

    pub fn deserialize_from(input: &mut impl std::io::Read) -> Result<Self> {
        let mut input = zstd::stream::read::Decoder::new(input)?;
        let result = bincode::deserialize_from(&mut input)?;
        Ok(result)
    }
}

#[cfg(test)]
mod test {
    use memory_image::Protection;

    #[test]
    fn test_serde() {
        use super::*;

        let mut classes = AddressClasses::new();
        classes
            .true_instructions
            .push(Interval::from_start_and_end(0, 10));
        classes
            .true_instructions
            .push(Interval::from_start_and_end(20, 30));
        classes.true_data.push(Interval::from_start_and_end(40, 50));

        let mut memory = MemoryImage::new();

        memory.add_region(0, Protection::READ_EXECUTE, vec![0; 60], "".to_string());

        let sample = ExecutableSample {
            memory,
            classes,
            source: None,
        };

        let mut output = Vec::new();
        sample.serialize_into(&mut output).unwrap();

        let sample2 = ExecutableSample::deserialize_from(&mut output.as_slice()).unwrap();

        assert_eq!(sample2.classes, sample.classes);
        assert_eq!(
            format!("{}", sample2.memory.dump()),
            format!("{}", sample.memory.dump())
        );
        assert_eq!(sample2.source, sample.source);
    }
}
