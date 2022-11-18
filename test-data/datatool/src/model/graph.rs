use crate::model::vocab::CodeVocab;
use crate::model::{InstructionFeature, Label, SupersetSample};
use ndarray::{Array1, Array2};
use ndarray_npy::NpzWriter;
use num_enum::IntoPrimitive;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Seek, Write};

#[derive(Serialize, Deserialize, IntoPrimitive, Copy, Clone)]
#[repr(u8)]
pub enum RelationType {
    Next = 0,
    Previous = 1,
    Overlap = 2,
    JumpTo = 3,
    JumpFrom = 4,
}

#[derive(Serialize, Deserialize)]
pub struct GraphSample {
    pub superset: Vec<(InstructionFeature, Option<Label>)>,
    pub graph: Vec<(usize, usize, RelationType)>,
    pub source: Option<String>,
}

impl GraphSample {
    pub fn new(superset: SupersetSample) -> Self {
        let mut graph = Vec::new();

        let mut index = HashMap::new();
        for (i, &(addr, _, _)) in superset.superset.iter().enumerate() {
            index.insert(addr, i);
        }

        for (i, &(addr, ref instr, _)) in superset.superset.iter().enumerate() {
            let next_addr = addr + instr.size as u32;
            if let Some(next) = index.get(&next_addr).cloned() {
                graph.push((i, next, RelationType::Next));
                graph.push((next, i, RelationType::Previous));
            }

            for j in addr..next_addr {
                if let Some(overlap) = index.get(&j).cloned() {
                    graph.push((i, overlap, RelationType::Overlap));
                    graph.push((overlap, i, RelationType::Overlap));
                }
            }

            if let Some(target) = instr.jump_target {
                if let Some(jump) = index.get(&target).cloned() {
                    graph.push((i, jump, RelationType::JumpTo));
                    graph.push((jump, i, RelationType::JumpFrom));
                }
            }
        }

        Self {
            superset: superset
                .superset
                .into_iter()
                .map(|(_addr, instr, label)| (instr, label))
                .collect(),
            graph,
            source: superset.source,
        }
    }

    pub fn to_npz<W: Write + Seek>(self, vocab: &CodeVocab, writer: W) -> anyhow::Result<()> {
        // let mut writer = zstd::stream::Encoder::new(
        //     writer, 6, /* tuned to be not too big (file), not too slow (compression) */
        // )?;

        // TODO: this is clearly not the most efficient solution, but it works ig

        // encode instructions
        let instruction_sizes = Array1::from_iter(self.superset.iter().map(|(i, _)| i.size));
        let instruction_codes =
            Array1::from_iter(self.superset.iter().map(|(i, _)| vocab[i.code] as u32));
        let instruction_labels = if self.superset.iter().all(|(_, l)| l.is_some()) {
            Some(Array1::from_iter(self.superset.iter().map(
                |(_, l)| match l.unwrap() {
                    Label::Code => 1u8,
                    Label::NotCode => 0u8,
                },
            )))
        } else {
            None
        };

        drop(self.superset);

        // encode relations
        let relation_types = Array1::from_iter(self.graph.iter().map(|&(_, _, t)| u8::from(t)));
        let relations = Array2::from_shape_vec(
            (self.graph.len(), 2),
            self.graph
                .iter()
                .flat_map(|&(a, b, _)| [a as u32, b as u32])
                .collect(),
        )
        .unwrap();
        drop(self.graph);

        let mut npz = NpzWriter::new_zstd_compressed(writer, Some(6));
        npz.add_array("instruction_sizes", &instruction_sizes)?;
        npz.add_array("instruction_codes", &instruction_codes)?;
        if let Some(instruction_labels) = instruction_labels {
            npz.add_array("instruction_labels", &instruction_labels)?;
        }
        npz.add_array("relation_types", &relation_types)?;
        npz.add_array("relations", &relations)?;
        npz.finish()?;

        Ok(())
    }
}
