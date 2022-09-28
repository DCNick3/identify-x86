use crate::loader::load_debug;
use crate::loader::load_elf;
use anyhow::Result;
use memory_image::MemoryImage;
use object::read::elf::ElfFile32;
use std::collections::HashSet;

pub struct ExecutableSample {
    memory: MemoryImage,
    true_instructions: HashSet<u32>,
    true_data: HashSet<u32>,
}

impl ExecutableSample {
    pub fn from_debian(executable: &ElfFile32, debug_info: &ElfFile32) -> Result<Self> {
        let memory = load_elf(executable)?;
        let debug = load_debug(debug_info)?;

        // println!("{}", memory.map());

        todo!()
    }
}
