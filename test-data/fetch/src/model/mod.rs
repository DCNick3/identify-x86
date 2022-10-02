pub mod interval_set;

use crate::loader::load_debug;
use crate::loader::load_elf;
use anyhow::Result;
use interval_set::IntervalSet;
use memory_image::MemoryImage;
use object::read::elf::ElfFile32;

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
    memory: MemoryImage,
    classes: AddressClasses,
}

impl ExecutableSample {
    pub fn from_debian(executable: &ElfFile32, debug_info: &ElfFile32) -> Result<Self> {
        let memory = load_elf(executable)?;
        let debug = load_debug(debug_info)?;

        // println!("{}", memory.map());

        todo!()
    }
}
