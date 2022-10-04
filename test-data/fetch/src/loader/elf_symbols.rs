use crate::model::AddressClasses;
use crate::Interval;
use anyhow::Result;
use object::read::elf::{ElfFile32, FileHeader};
use object::{elf, Object};

pub fn dump_elf_symbols(
    // base_addr: u32,
    elf: &ElfFile32,
) -> Result<AddressClasses> {
    let e = elf.endianness();

    let sections = elf.raw_header().sections(e, elf.data())?;
    let symbol_table = sections.symbols(e, elf.data(), elf::SHT_SYMTAB)?;

    let mut classes = AddressClasses::new();

    for symbol in symbol_table.iter() {
        // skup undefined symbols
        if symbol.st_shndx.get(e) == elf::SHN_UNDEF {
            continue;
        }

        let address = symbol.st_value.get(e);
        let size = symbol.st_size.get(e);
        // let name = std::str::from_utf8(symbol.name(e, symbol_table.strings())?)?;
        let kind = symbol.st_type();

        // skip uninteresting symbols
        if matches!(
            kind,
            elf::STT_NOTYPE | elf::STT_FILE | elf::STT_SECTION | elf::STT_TLS
        ) {
            continue;
        }

        match kind {
            elf::STT_OBJECT | elf::STT_COMMON => classes
                .true_data
                .push(Interval::from_start_and_len(address, size)),
            elf::STT_FUNC => classes
                .true_instructions
                .push(Interval::from_start_and_len(address, size)),
            kind => panic!("Unknown symbol type: {}", kind),
        };
    }

    Ok(classes)
}
