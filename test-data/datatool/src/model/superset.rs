use crate::model::ExecutableSample;
use iced_x86::{Code, DecoderOptions};
use itertools::Itertools;
use parquet::basic::Compression;
use parquet::file::metadata::KeyValue;
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

#[derive(Serialize, Deserialize, Copy, Clone)]
pub struct InstructionFeature {
    pub size: u8,
    pub code: Code,
    pub jump_target: Option<u32>,
}

impl From<iced_x86::Instruction> for InstructionFeature {
    fn from(instruction: iced_x86::Instruction) -> Self {
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
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SupersetSample {
    pub superset: Vec<(u32, InstructionFeature, Option<Label>)>,
    pub source: Option<String>,
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

        SupersetSample {
            superset,
            source: sample.source.map(|v| format!("{:?}", v)),
        }
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

        let mut metadata = Vec::new();
        if let Some(src) = self.source {
            metadata.push(KeyValue::new("source".to_string(), src))
        }

        let schema = records.schema()?;
        let props = Arc::new(
            WriterProperties::builder()
                .set_key_value_metadata(Some(metadata))
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
