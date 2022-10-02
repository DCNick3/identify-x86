use crate::model::interval_set::Interval;
use crate::model::AddressClasses;
use anyhow::Result;
use pdb::{AddressMap, FallibleIterator, PDB};
use std::collections::BTreeSet;

struct Ranges {
    instruction_ranges: Vec<(u32, u32)>,
    data_locations: BTreeSet<u32>,
}

fn extract_symbol(
    ranges: &mut Ranges,
    address_map: &AddressMap<'_>,
    symbol: &pdb::Symbol<'_>,
) -> pdb::Result<()> {
    match symbol.parse()? {
        pdb::SymbolData::Data(data) => {
            if let Some(addr) = data.offset.to_rva(address_map) {
                ranges.data_locations.insert(addr.0);
            }
        }
        pdb::SymbolData::Procedure(data) => {
            if let Some(addr) = data.offset.to_rva(address_map) {
                ranges.instruction_ranges.push((addr.0, data.len as u32));
            }
        }
        pdb::SymbolData::Trampoline(data) => {
            if let Some(addr) = data.thunk.to_rva(address_map) {
                ranges.instruction_ranges.push((addr.0, data.size as u32));
            }
        }
        _ => {
            // ignore everything else
        }
    }

    Ok(())
}

fn walk_symbols(
    ranges: &mut Ranges,
    address_map: &AddressMap<'_>,
    mut symbols: pdb::SymbolIter<'_>,
) -> pdb::Result<()> {
    while let Some(symbol) = symbols.next()? {
        let _ = extract_symbol(ranges, address_map, &symbol);
    }

    Ok(())
}

pub fn dump_pdb<'a, T: pdb::Source<'a> + 'a>(
    base_addr: u32,
    pdb: &mut PDB<'a, T>,
) -> Result<AddressClasses> {
    let mut ranges = Ranges {
        instruction_ranges: Vec::new(),
        data_locations: BTreeSet::new(),
    };

    let address_map = pdb.address_map()?;

    let dbi = pdb.debug_information()?;
    let mut modules = dbi.modules()?;
    while let Some(module) = modules.next()? {
        let info = match pdb.module_info(&module)? {
            Some(info) => info,
            None => continue,
        };

        walk_symbols(&mut ranges, &address_map, info.symbols()?)?;
    }

    let instruction_starts = ranges
        .instruction_ranges
        .iter()
        .map(|&(a, _)| a)
        .collect::<BTreeSet<_>>();

    let mut classes = AddressClasses::new();

    let mut instr_iter = instruction_starts.iter().peekable();
    for &addr in ranges.data_locations.iter() {
        while let Some(&&next_instr) = instr_iter.peek() {
            if next_instr >= addr {
                break;
            }
            instr_iter.next();
        }

        if let Some(&&next_instr) = instr_iter.peek() {
            let len = next_instr - addr;

            classes
                .true_data
                .push(Interval::from_start_and_len(addr, len));
        }
    }

    for (addr, len) in ranges.instruction_ranges {
        classes
            .true_instructions
            .push(Interval::from_start_and_len(addr, len));
    }

    classes.relocate(base_addr);

    Ok(classes)
}
