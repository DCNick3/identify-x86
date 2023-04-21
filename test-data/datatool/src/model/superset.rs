use crate::model::{ExecutableSample, GraphSample};
use bitflags::bitflags;
use enum_map::Enum;
use iced_x86::{Code, DecoderOptions, InstructionInfoFactory, OpAccess, RflagsBits};
use itertools::Itertools;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use parquet::file::writer::SerializedFileWriter;
use parquet::record::RecordWriter;
use parquet_derive::ParquetRecordWriter;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Write;
use std::sync::Arc;

#[derive(Serialize, Deserialize, Copy, Clone)]
pub enum Label {
    Code,
    NotCode,
}

#[derive(Enum, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum UsedRegister {
    Eax = 0,
    Ecx = 1,
    Edx = 2,
    Ebx = 3,
    Esp = 4,
    Ebp = 5,
    Esi = 6,
    Edi = 7,

    Cf = 8,
    Pf = 9,
    Af = 10,
    Zf = 11,
    Sf = 12,
}

bitflags! {
    #[derive(Debug, Copy, Clone, Serialize, Deserialize)]
    pub struct RegisterSet: u16 {
        // track only full-size registers
        const EAX = 1 << UsedRegister::Eax as u16;
        const ECX = 1 << UsedRegister::Ecx as u16;
        const EDX = 1 << UsedRegister::Edx as u16;
        const EBX = 1 << UsedRegister::Ebx as u16;
        const ESP = 1 << UsedRegister::Esp as u16;
        const EBP = 1 << UsedRegister::Ebp as u16;
        const ESI = 1 << UsedRegister::Esi as u16;
        const EDI = 1 << UsedRegister::Edi as u16;

        // track status flags separately
        const CF = 1 << UsedRegister::Cf as u16;
        const PF = 1 << UsedRegister::Pf as u16;
        const AF = 1 << UsedRegister::Af as u16;
        const ZF = 1 << UsedRegister::Zf as u16;
        const SF = 1 << UsedRegister::Sf as u16;
    }
}

impl RegisterSet {
    #[inline]
    pub fn iter_used_registers(&self) -> RegisterSetIterator {
        RegisterSetIterator {
            set: *self,
            current: 0,
        }
    }

    pub fn from_rflags(rflags: u32) -> Self {
        let mut set = RegisterSet::empty();
        if rflags & RflagsBits::CF as u32 != 0 {
            set |= RegisterSet::CF;
        }
        if rflags & RflagsBits::PF as u32 != 0 {
            set |= RegisterSet::PF;
        }
        if rflags & RflagsBits::AF as u32 != 0 {
            set |= RegisterSet::AF;
        }
        if rflags & RflagsBits::ZF as u32 != 0 {
            set |= RegisterSet::ZF;
        }
        if rflags & RflagsBits::SF as u32 != 0 {
            set |= RegisterSet::SF;
        }
        set
    }
}

impl From<iced_x86::Register> for RegisterSet {
    fn from(reg: iced_x86::Register) -> Self {
        use iced_x86::Register::*;
        match reg.full_register32() {
            EAX => RegisterSet::EAX,
            ECX => RegisterSet::ECX,
            EDX => RegisterSet::EDX,
            EBX => RegisterSet::EBX,
            ESP => RegisterSet::ESP,
            EBP => RegisterSet::EBP,
            ESI => RegisterSet::ESI,
            EDI => RegisterSet::EDI,
            _ => RegisterSet::empty(),
        }
    }
}

impl From<UsedRegister> for RegisterSet {
    fn from(reg: UsedRegister) -> Self {
        use UsedRegister::*;
        match reg {
            Eax => RegisterSet::EAX,
            Ecx => RegisterSet::ECX,
            Edx => RegisterSet::EDX,
            Ebx => RegisterSet::EBX,
            Esp => RegisterSet::ESP,
            Ebp => RegisterSet::EBP,
            Esi => RegisterSet::ESI,
            Edi => RegisterSet::EDI,
            Cf => RegisterSet::CF,
            Pf => RegisterSet::PF,
            Af => RegisterSet::AF,
            Zf => RegisterSet::ZF,
            Sf => RegisterSet::SF,
        }
    }
}

pub struct RegisterSetIterator {
    set: RegisterSet,
    current: usize,
}

impl Iterator for RegisterSetIterator {
    type Item = UsedRegister;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= UsedRegister::LENGTH {
            return None;
        }
        // let current = UsedRegister::from_usize(self.current);
        let current: UsedRegister = unsafe { std::mem::transmute(self.current as u8) };
        self.current = self.current + 1;
        if self.set.contains(current.into()) {
            Some(current)
        } else {
            self.next()
        }
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub struct InstructionFeature {
    pub size: u8,
    pub code: Code,
    pub jump_target: Option<u32>,
    pub falls_through: bool,
    pub uses: RegisterSet,
    pub defines: RegisterSet,
}

impl From<iced_x86::Instruction> for InstructionFeature {
    fn from(instruction: iced_x86::Instruction) -> Self {
        // TODO: reuse the InstructionInfoFactory (it allocates)
        let mut factory = InstructionInfoFactory::new();
        let instr_info = factory.info_options(
            &instruction,
            iced_x86::InstructionInfoOptions::NO_MEMORY_USAGE,
        );

        let defines = instr_info
            .used_registers()
            .iter()
            .filter(|r| {
                matches!(
                    r.access(),
                    OpAccess::Write
                        | OpAccess::ReadWrite
                        | OpAccess::CondWrite
                        | OpAccess::ReadCondWrite
                )
            })
            .map(|r| r.register().into())
            .fold(RegisterSet::empty(), |acc, v| acc | v)
            | RegisterSet::from_rflags(instruction.rflags_modified());
        let uses = instr_info
            .used_registers()
            .iter()
            .filter(|r| {
                matches!(
                    r.access(),
                    OpAccess::Read
                        | OpAccess::ReadWrite
                        | OpAccess::CondRead
                        | OpAccess::ReadCondWrite
                )
            })
            .map(|r| r.register().into())
            .fold(RegisterSet::empty(), |acc, v| acc | v)
            | RegisterSet::from_rflags(instruction.rflags_read()); // TODO: maybe we want to handle the "undefined" as a special case

        InstructionFeature {
            size: instruction.len() as u8,
            code: instruction.code(),
            jump_target: if instruction
                .op_kinds()
                .contains(&iced_x86::OpKind::NearBranch32)
            {
                Some(instruction.near_branch32())
            } else {
                None
            },
            falls_through: !matches!(
                instruction.flow_control(),
                iced_x86::FlowControl::UnconditionalBranch
                    | iced_x86::FlowControl::IndirectBranch
                    | iced_x86::FlowControl::Return
                    | iced_x86::FlowControl::Exception
            ),
            defines,
            uses,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SupersetSample {
    pub superset: Vec<(u32, InstructionFeature, Option<Label>)>,
}

impl SupersetSample {
    pub fn new(sample: ExecutableSample) -> Self {
        let mut instruction_addresses = HashSet::new();

        // the assumption here is that inside the interval marked as code there is no gaps
        // this __should__ be true if the compiler is sane
        for interval in sample.classes.true_instructions.iter() {
            let mut decoder = iced_x86::Decoder::new(
                32,
                &sample.memory.execute_all_at(interval.start())[..interval.len() as usize],
                DecoderOptions::NONE,
            );
            decoder.set_ip(interval.start() as u64);

            loop {
                let instr = decoder.decode();
                if instr.is_invalid() {
                    break;
                }
                instruction_addresses.insert(instr.ip32());
            }
        }

        let mut superset = Vec::new();
        for item in sample.memory.iter() {
            let mut decoder = iced_x86::Decoder::new(32, &item.data, 0);

            for address in item.addr..item.end() {
                decoder
                    .set_position((address - item.addr) as usize)
                    .unwrap();
                decoder.set_ip(address as u64);
                let instruction = decoder.decode();
                let instruction = InstructionFeature::from(instruction);

                let label = Some(if instruction_addresses.contains(&address) {
                    Label::Code
                } else {
                    Label::NotCode
                });
                superset.push((address, instruction, label));
            }
        }

        SupersetSample { superset }
    }

    pub fn into_graph(self) -> GraphSample {
        GraphSample::new(self)
    }

    pub fn to_parquet<W: Write>(self, writer: W) -> anyhow::Result<()> {
        #[derive(ParquetRecordWriter)]
        struct Record {
            pub addr: i32,
            pub size: i32,
            pub code: i32,
            pub label: Option<bool>,
        }

        let records = self
            .superset
            .into_iter()
            .map(|(addr, instr, label)| Record {
                addr: addr.try_into().unwrap(),
                size: instr.size as i32,
                code: instr.code as u16 as i32,
                label: label.map(|v| match v {
                    Label::Code => true,
                    Label::NotCode => false,
                }),
            })
            .collect::<Vec<_>>();

        let records = records.as_slice();

        let schema = records.schema()?;
        let props = Arc::new(
            WriterProperties::builder()
                // .set_key_value_metadata(Some(metadata))
                .set_compression(Compression::ZSTD)
                .build(),
        );
        let mut writer = SerializedFileWriter::new(writer, schema, props)?;
        let mut row_group = writer.next_row_group()?;

        records.write_to_row_group(&mut row_group)?;

        row_group.close()?;
        writer.close()?;

        Ok(())
    }
}
