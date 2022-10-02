mod dwarf;
mod pdb;

use anyhow::Result;
use memory_image::{MemoryImage, Protection};
use object::elf::{PF_R, PF_W, PF_X};
use object::pe::{IMAGE_SCN_MEM_EXECUTE, IMAGE_SCN_MEM_READ, IMAGE_SCN_MEM_WRITE};
use object::{Object, ObjectSegment, SegmentFlags};

pub use self::pdb::dump_pdb;
pub use dwarf::load_dwarf;

fn flags_to_protection(flags: SegmentFlags) -> Protection {
    match flags {
        SegmentFlags::None => {
            panic!("SegmentFlags::None does not make sense");
        }
        SegmentFlags::Elf { p_flags } => {
            let flags = p_flags & 0x7;

            if flags == PF_X {
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
            }
        }
        SegmentFlags::MachO { .. } => {
            todo!("MachO segment flags")
        }
        SegmentFlags::Coff { characteristics } => {
            let flags = characteristics
                & (IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_WRITE | IMAGE_SCN_MEM_EXECUTE);

            if flags == IMAGE_SCN_MEM_EXECUTE {
                Protection::EXECUTE
            } else if flags == IMAGE_SCN_MEM_WRITE {
                Protection::WRITE
            } else if flags == (IMAGE_SCN_MEM_WRITE | IMAGE_SCN_MEM_EXECUTE) {
                Protection::WRITE_EXECUTE
            } else if flags == IMAGE_SCN_MEM_READ {
                Protection::READ
            } else if flags == (IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_EXECUTE) {
                Protection::READ_EXECUTE
            } else if flags == (IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_WRITE) {
                Protection::READ_WRITE
            } else if flags == (IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_WRITE | IMAGE_SCN_MEM_EXECUTE) {
                Protection::READ_WRITE_EXECUTE
            } else {
                panic!("Invalid segment access flags: {}", flags);
            }
        }
        _ => {
            todo!("Getting protection for {:?}", flags)
        }
    }
}

pub fn load_executable<'data: 'file, 'file>(
    object: &'file impl Object<'data, 'file>,
) -> Result<MemoryImage> {
    let mut res = MemoryImage::new();

    // do we want to give some special handling to the dynamic executables?
    // let is_dyn = elf.raw_header().e_type.get(elf.endian()) == ET_DYN;

    for segment in object.segments() {
        let addr = segment.address() as u32;
        let mut data = segment.data().unwrap().to_vec();

        while (data.len() as u64) < segment.size() {
            data.push(0)
        }

        let prot = flags_to_protection(segment.flags());

        res.add_region(addr, prot, data.to_vec(), "".to_string());
    }

    Ok(res)
}
