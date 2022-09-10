use anyhow::{Context, Result};
use gimli::{BaseAddresses, EndianReader, Section, UnwindSection};
use memory_image::{MemoryImage, Protection};
use object::elf::{ET_DYN, PF_R, PF_W, PF_X};
use object::read::elf::ElfFile32;
use object::{Object, ObjectSection, ObjectSegment, SegmentFlags};
use std::borrow;
use std::sync::Arc;

fn get_base_address(is_dynamic: bool) -> u32 {
    if is_dynamic {
        0x40000
    } else {
        0
    }
}

pub fn load_elf(elf: &ElfFile32) -> Result<MemoryImage> {
    let mut res = MemoryImage::new();

    let is_dyn = elf.raw_header().e_type.get(elf.endian()) == ET_DYN;
    let addr_offset = get_base_address(is_dyn);

    // TODO: apply relocations

    for segment in elf.segments() {
        let addr = segment.address() as u32 + addr_offset;
        let mut data = segment.data().unwrap().to_vec();

        // let size = if segment.align() == 0 {
        //     segment.size()
        // } else {
        //     segment.size()
        //         + (segment.align() - (segment.size() % segment.align())) % segment.align()
        // };

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

fn load_gimli(elf: &ElfFile32) -> Result<()> {
    let endian = if elf.is_little_endian() {
        gimli::RunTimeEndian::Little
    } else {
        gimli::RunTimeEndian::Big
    };

    // Load a section and return as `Cow<[u8]>`.
    let load_section = |id: gimli::SectionId| -> Result<EndianReader<gimli::RunTimeEndian, Arc<[u8]>>, gimli::Error> {
        let bytes = match elf.section_by_name(id.name()) {
            Some(ref section) => section
                .uncompressed_data()
                .unwrap_or(borrow::Cow::Borrowed(&[][..])),
            None => borrow::Cow::Borrowed(&[][..]),
        };

        let bytes: Arc<[u8]> = Arc::from(bytes.as_ref());

        Ok(EndianReader::new(bytes, endian))
    };

    let base_addresses = BaseAddresses::default();

    let mut eh_frame = gimli::EhFrame::load(&load_section).context("Loading .eh_frame section")?;
    eh_frame.set_address_size(4);
    eh_frame.entries(&base_addresses);

    Ok(())
}
