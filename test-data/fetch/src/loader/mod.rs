mod dwarf;
mod pdb;

use anyhow::Result;
use memory_image::{MemoryImage, Protection};
use object::elf::{PF_R, PF_W, PF_X};
use object::read::elf::ElfFile32;
use object::read::pe::PeFile32;
use object::{Object, ObjectSegment, SegmentFlags};

pub use self::pdb::dump_pdb;
pub use dwarf::{load_debug, DebugInfo};

pub fn load_elf(elf: &ElfFile32) -> Result<MemoryImage> {
    let mut res = MemoryImage::new();

    // do we want to give some special handling to the dynamic executables?
    // let is_dyn = elf.raw_header().e_type.get(elf.endian()) == ET_DYN;

    for segment in elf.segments() {
        let addr = segment.address() as u32;
        let mut data = segment.data().unwrap().to_vec();

        while (data.len() as u64) < segment.size() {
            data.push(0)
        }

        let flags = match segment.flags() {
            SegmentFlags::Elf { p_flags } => p_flags,
            _ => unreachable!(),
        } & 0x7;

        let prot = if flags == PF_X {
            Protection::EXECUTE
        } else if flags == PF_W {
            Protection::WRITE
        } else if flags == (PF_W | PF_X) {
            Protection::WRITE_EXECUTE
        } else if flags == PF_R {
            Protection::READ
        } else if flags == (PF_R | PF_X) {
            Protection::READ_EXECUTE
        } else if flags == (PF_R | PF_W) {
            Protection::READ_WRITE
        } else if flags == (PF_R | PF_W | PF_X) {
            Protection::READ_WRITE_EXECUTE
        } else {
            panic!("Invalid segment access flags: {}", flags);
        };

        res.add_region(addr, prot, data.to_vec(), "".to_string());
    }

    Ok(res)
}

pub fn load_pe(pe: &PeFile32) -> Result<MemoryImage> {
    let mut res = MemoryImage::new();

    for segment in pe.segments() {
        let addr = segment.address() as u32;
        let mut data = segment.data().unwrap().to_vec();

        while (data.len() as u64) < segment.size() {
            data.push(0)
        }

        let flags = match segment.flags() {
            SegmentFlags::Elf { p_flags } => p_flags,
            _ => unreachable!(),
        } & 0x7;

        let prot = if flags == PF_X {
            Protection::EXECUTE
        } else if flags == PF_W {
            Protection::WRITE
        } else if flags == (PF_W | PF_X) {
            Protection::WRITE_EXECUTE
        } else if flags == PF_R {
            Protection::READ
        } else if flags == (PF_R | PF_X) {
            Protection::READ_EXECUTE
        } else if flags == (PF_R | PF_W) {
            Protection::READ_WRITE
        } else if flags == (PF_R | PF_W | PF_X) {
            Protection::READ_WRITE_EXECUTE
        } else {
            panic!("Invalid segment access flags: {}", flags);
        };

        res.add_region(addr, prot, data.to_vec(), "".to_string());
    }

    Ok(res)
}
