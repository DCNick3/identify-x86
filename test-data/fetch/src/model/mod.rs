pub mod interval_set;

use crate::loader::load_dwarf;
use crate::loader::load_executable;
use crate::{dump_pdb, Interval};
use anyhow::{bail, Result};
use interval_set::IntervalSet;
use itertools::Itertools;
use memory_image::MemoryImage;
use object::read::elf::ElfFile32;
use object::read::pe::PeFile32;
use pdb::PDB;

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
        for (interval, kind) in data {
            use std::fmt::Write;
            writeln!(
                result,
                "0x{:08x} - 0x{:08x} {}",
                interval.start(),
                interval.end(),
                kind
            )
            .unwrap();
        }

        result
    }
}

pub struct ExecutableSample {
    pub memory: MemoryImage,
    pub classes: AddressClasses,
}

impl ExecutableSample {
    pub fn from_debian(executable: &ElfFile32, debug_info: &ElfFile32) -> Result<Self> {
        let _memory = load_executable(executable)?;
        let _debug = load_dwarf(debug_info)?;

        // TODO: we should use symbol information to determine the code/data locations

        todo!()
    }

    pub fn from_pe<'s, S: std::io::Read + std::io::Seek + std::fmt::Debug + 's>(
        executable: &PeFile32,
        debug_info: &mut PDB<'s, S>,
    ) -> Result<Self> {
        use object::Object;

        if let Some(pdb_info) = executable.pdb_info()? {
            let provided_guid = debug_info.pdb_information()?.guid;
            let expected_guid = uuid::Uuid::from_slice(&pdb_info.guid())?;
            if provided_guid != expected_guid {
                bail!(
                    "PDB GUID mismatch: expected {:?}, got {:?}",
                    expected_guid,
                    provided_guid
                );
            }
        } else {
            bail!("PE file does not contain PDB info");
        }

        let memory = load_executable(executable)?;
        let mut classes = dump_pdb(
            executable.relative_address_base().try_into().unwrap(),
            debug_info,
        )?;

        let exe_item = memory
            .iter()
            .filter(|v| v.protection.contains(memory_image::Protection::EXECUTE))
            .exactly_one()
            .map_err(|_| anyhow::anyhow!("Either no executable section or more than one"))?;

        let exe_range = Interval::from_start_and_end(exe_item.addr, exe_item.end());

        classes.filter_to(exe_range);

        // preserve only the executable section
        let memory = MemoryImage::from_iter(vec![exe_item].into_iter());

        Ok(ExecutableSample { memory, classes })
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
}
