use anyhow::{anyhow, bail, Context, Result};
use gimli::{
    AttributeValue, DW_AT_high_pc, DW_AT_language, DW_AT_location, DW_AT_low_pc, DW_AT_name,
    DW_AT_ranges, DW_LANG_Mips_Assembler, EndianReader, EvaluationResult, Location, Piece,
    RunTimeEndian,
};
use object::read::elf::ElfFile32;
use object::{Object, ObjectSection};
use std::borrow;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{info, warn};

type Reader = EndianReader<RunTimeEndian, Arc<[u8]>>;
type Dwarf = gimli::Dwarf<Reader>;
type RangeLists = gimli::RangeLists<Reader>;
type Unit = gimli::Unit<Reader>;
type EntriesTreeNode<'abbrev, 'unit, 'tree> = gimli::EntriesTreeNode<'abbrev, 'unit, 'tree, Reader>;
type DebuggingInformationEntry<'abbrev, 'unit> =
    gimli::DebuggingInformationEntry<'abbrev, 'unit, Reader>;

pub struct DwarfInfo {
    function_ranges: Vec<AddressRange>,
    data_range: AddressRange,
}

impl DwarfInfo {
    fn new() -> Self {
        Self {
            function_ranges: Default::default(),
            data_range: Default::default(),
        }
    }
}

// TODO: devise a smart data structure or smth
// (this is very suboptimal)
struct AddressRange {
    ranges: Vec<(u32, u32)>,
}

impl AddressRange {
    pub fn new() -> Self {
        Self { ranges: Vec::new() }
    }

    pub fn from_simple_range(start: u32, end: u32) -> Self {
        Self {
            ranges: vec![(start, end)],
        }
    }

    pub fn merge(&mut self, other: &Self) {
        // TODO: this is very suboptimal and probably not even correct
        self.ranges.extend(other.ranges.iter());
    }

    pub fn add_range(&mut self, start: u32, end: u32) {
        self.merge(&Self::from_simple_range(start, end));
    }

    pub fn try_from_dwarf_entry(
        dwarf: &Dwarf,
        unit: &Unit,
        entry: &DebuggingInformationEntry,
    ) -> Result<Option<Self>> {
        if let Some(low_pc) = entry.attr(DW_AT_low_pc)? {
            let high_pc = entry.attr(DW_AT_high_pc)?.unwrap();

            let low_pc: u32 = if let AttributeValue::Addr(addr) = low_pc.value() {
                addr.try_into().unwrap()
            } else {
                unreachable!();
            };
            let high_pc: u32 = high_pc.udata_value().unwrap().try_into().unwrap();
            let high_pc = low_pc + high_pc;

            Ok(Some(Self::from_simple_range(low_pc, high_pc)))
        } else if let Some(fn_ranges) = entry.attr(DW_AT_ranges)? {
            let sec_offset = if let AttributeValue::RangeListsRef(offset) = fn_ranges.value() {
                offset
            } else {
                unreachable!();
            };
            let mut ranges = dwarf
                .ranges(unit, dwarf.ranges_offset_from_raw(unit, sec_offset))
                .context("Getting ranges for the subroutine")?;

            let mut result = Self::new();

            while let Some(range) = ranges.next()? {
                result.add_range(
                    range.begin.try_into().unwrap(),
                    range.end.try_into().unwrap(),
                );
            }

            Ok(Some(result))
        } else {
            Ok(None)
        }
    }
}

impl Default for AddressRange {
    fn default() -> Self {
        AddressRange::new()
    }
}

fn compute_location(unit: &Unit, entry: &DebuggingInformationEntry) -> Result<Option<u32>> {
    Ok(if let Some(location) = entry.attr(DW_AT_location)? {
        let location = location.value().exprloc_value().unwrap();

        let mut eval = location.evaluation(unit.encoding());
        let mut result = eval.evaluate()?;
        while result != EvaluationResult::Complete {
            match result {
                EvaluationResult::RequiresRelocatedAddress(addr) => {
                    result = eval.resume_with_relocated_address(addr)?;
                }
                _ => unimplemented!(),
            };
        }

        let res = eval.result();

        if let [Piece {
            location: Location::Address { address },
            ..
        }] = res.as_slice()
        {
            let addr: u32 = (*address).try_into().unwrap();
            Some(addr)
        } else {
            unreachable!()
        }
    } else {
        None
    })
}

fn collect_data(
    res: &mut AddressRange,
    dwarf: &Dwarf,
    unit: &Unit,
    tree: EntriesTreeNode,
) -> Result<()> {
    let mut children = tree.children();
    while let Some(child) = children
        .next()
        .context("Reading tree children to collect functions")?
    {
        let entry = child.entry();
        use gimli::constants::*;
        #[allow(non_upper_case_globals)]
        match entry.tag() {
            DW_TAG_variable => {
                if entry.attr(DW_AT_declaration)?.is_some() {
                    // skip declaration, they do not take space in memory
                    continue;
                }

                let addr = compute_location(unit, entry)?.unwrap();

                if let Some(spec) = entry.attr(DW_AT_specification)? {
                    let v = if let AttributeValue::UnitRef(v) = spec.value() {
                        v
                    } else {
                        unreachable!()
                    };
                    let mut spec = unit.entries_at_offset(v)?;
                    spec.next_entry()?.unwrap();
                    let spec = spec.current().unwrap();


                    let mut attrs = spec.attrs();
                    while let Some(attr) = attrs.next()? {
                        println!("    {}", attr.name());
                    }
                    todo!()
                }
            }
            DW_TAG_class_type => {
                // TODO: I __think__ the static variables (we want to collect them) are represented as globals
                // need to check though
                todo!("Handle classes")
            }
            // skip these, they can't define data
            DW_TAG_subprogram
            | DW_TAG_typedef
            // type modifiers
            | DW_TAG_atomic_type
            | DW_TAG_const_type
            | DW_TAG_immutable_type
            | DW_TAG_packed_type
            | DW_TAG_pointer_type
            | DW_TAG_reference_type
            | DW_TAG_restrict_type
            | DW_TAG_rvalue_reference_type
            | DW_TAG_shared_type
            | DW_TAG_volatile_type
            // the types themselves
            | DW_TAG_base_type
            | DW_TAG_structure_type
            | DW_TAG_array_type
            | DW_TAG_subroutine_type
            | DW_TAG_enumeration_type
            | DW_TAG_union_type
            // DWARF procedure... Not interesting
            | DW_TAG_dwarf_procedure
            => {}
            tag => {
                bail!("Unknown DWARF tag: {}", tag)
            }
        }
    }
    Ok(())
}

fn collect_functions(
    res: &mut Vec<AddressRange>,
    dwarf: &Dwarf,
    unit: &Unit,
    tree: EntriesTreeNode,
) -> Result<()> {
    let mut children = tree.children();
    while let Some(child) = children
        .next()
        .context("Reading tree children to collect functions")?
    {
        let entry = child.entry();
        use gimli::constants::*;
        #[allow(non_upper_case_globals)]
        match entry.tag() {
            DW_TAG_subprogram => {
                // let mut attrs = entry.attrs();
                // while let Some(attr) = attrs.next()? {
                //     println!("    {} {:?}", attr.name(), attr.value());
                // }
                if let Some(location) = AddressRange::try_from_dwarf_entry(dwarf, unit, entry).context("Getting function location")? {
                    res.push(location);
                }
            }
            DW_TAG_class_type => {
                todo!("Handle classes")
            }
            // skip these, they can't define functions
            DW_TAG_variable
            | DW_TAG_typedef
            // type modifiers
            | DW_TAG_atomic_type
            | DW_TAG_const_type
            | DW_TAG_immutable_type
            | DW_TAG_packed_type
            | DW_TAG_pointer_type
            | DW_TAG_reference_type
            | DW_TAG_restrict_type
            | DW_TAG_rvalue_reference_type
            | DW_TAG_shared_type
            | DW_TAG_volatile_type
            // the types themselves
            | DW_TAG_base_type
            | DW_TAG_structure_type
            | DW_TAG_array_type
            | DW_TAG_subroutine_type
            | DW_TAG_enumeration_type
            | DW_TAG_union_type
            // DWARF procedure... Not interesting
            | DW_TAG_dwarf_procedure
            => {}
            tag => {
                bail!("Unknown DWARF tag: {}", tag)
            }
        }
    }
    Ok(())
}

fn load_unit(res: &mut DwarfInfo, dwarf: &Dwarf, unit: &Unit) -> Result<()> {
    let mut entries = unit.entries_tree(None).context("Getting entries tree")?;
    let root = entries.root().context("Getting root of debug entries")?;

    let entry = root.entry();

    println!("<{:x}> {}", entry.offset().0, entry.tag());
    let name = dwarf
        .attr_string(
            &unit,
            entry
                .attr(DW_AT_name)?
                .ok_or_else(|| anyhow!("Compile unit without a name? bollocks!"))?
                .value(),
        )
        .context("Getting a compile unit name")?;
    let name =
        std::str::from_utf8(name.bytes()).context("Converting compile unit name to a string")?;

    if let Some(lang) = entry.attr(DW_AT_language)? {
        if let AttributeValue::Language(lang) = lang.value() {
            if lang == DW_LANG_Mips_Assembler {
                info!("Skipping the assembly compile unit {}", name);
                return Ok(());
            }
        } else {
            unreachable!()
        }
    } else {
        warn!("Could not determine language")
    }

    collect_functions(&mut res.function_ranges, dwarf, unit, root)
        .context("Collecting functions")?;

    let root = entries.root().context("Getting root of debug entries")?;
    collect_data(&mut res.data_range, dwarf, unit, root).context("Collecting data")?;

    Ok(())
}

pub fn load_dwarf(elf: &ElfFile32) -> Result<DwarfInfo> {
    let endian = if elf.is_little_endian() {
        RunTimeEndian::Little
    } else {
        RunTimeEndian::Big
    };

    // Load a section and return as `Cow<[u8]>`.
    let load_section =
        |id: gimli::SectionId| -> Result<EndianReader<RunTimeEndian, Arc<[u8]>>, gimli::Error> {
            let bytes = match elf.section_by_name(id.name()) {
                Some(ref section) => section
                    .uncompressed_data()
                    .unwrap_or(borrow::Cow::Borrowed(&[][..])),
                None => borrow::Cow::Borrowed(&[][..]),
            };

            let bytes: Arc<[u8]> = Arc::from(bytes.as_ref());

            Ok(EndianReader::new(bytes, endian))
        };

    let mut res = DwarfInfo::new();

    let dwarf = Dwarf::load(&load_section).context("Loading DWARF info")?;

    let mut units = dwarf.units();
    while let Some(unit) = units.next().context("Getting a next compile unit")? {
        let unit = dwarf.unit(unit).context("Parsing a DWARF compile unit")?;
        load_unit(&mut res, &dwarf, &unit).context("Loading a debug info compile unit")?;
    }

    Ok(res)
}
