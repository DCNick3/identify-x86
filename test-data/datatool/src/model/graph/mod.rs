#[allow(unused)]
mod dummy_single_vec;
mod single_vec;

// this provides marginal improvement in memory usage
use single_vec::SingleVec;
// use dummy_single_vec::SingleVec;

use crate::model::superset::UsedRegister;
use crate::model::vocab::CodeVocab;
use crate::model::{InstructionFeature, Label, SupersetSample};
use arrayvec::ArrayVec;
use enum_map::EnumMap;
use ndarray::{Array1, Array2};
use ndarray_npy::NpzWriter;
use num_enum::IntoPrimitive;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::io::{Seek, Write};

#[derive(
    Serialize, Deserialize, IntoPrimitive, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[repr(u8)]
pub enum RelationType {
    Next = 0,
    Previous = 1,
    Overlap = 2,
    JumpTo = 3,
    JumpFrom = 4,
    DataDependency = 5,
    DataDependent = 6,
}

// stores the indices of the latest definition of a register
type DataDepState = EnumMap<UsedRegister, SingleVec>;
type Index32 = u32;
type Address32 = u32;

fn get_instr_out_edges(
    superset: &[(Address32, InstructionFeature, Option<Label>)],
    superset_index: &FxHashMap<Address32, Index32>,
    index: usize,
) -> ArrayVec<Index32, 2> {
    let (addr, instr, _) = superset[index];
    let next_addr = addr + instr.size as Address32;

    let out_edges = None
        .into_iter()
        // the next instruction
        .chain(
            instr
                .falls_through
                .then(|| superset_index.get(&next_addr).cloned())
                .flatten(),
        )
        // the jump target
        .chain(
            instr
                .jump_target
                .and_then(|target| superset_index.get(&target).cloned()),
        )
        .rev()
        .collect();

    out_edges
}

fn toposort(
    superset: &[(Address32, InstructionFeature, Option<Label>)],
    superset_index: &FxHashMap<Address32, Index32>,
) -> Vec<Index32> {
    struct BacktrackStackItem {
        index: Index32,
        iter: ArrayVec<Index32, 2>,
    }

    let len = superset.len();
    let mut stack = Vec::new();
    let mut is_in_stack = vec![false; len];
    let mut was_visited = vec![false; len];
    let mut result = Vec::new();

    // TODO: maybe find a better ordering?
    // currently, this results in just in 0..len being returned from the function
    for start_index in (0..len).rev() {
        if was_visited[start_index] {
            continue;
        }

        stack.push(BacktrackStackItem {
            index: start_index as Index32,
            iter: get_instr_out_edges(superset, superset_index, start_index),
        });

        while let Some(mut item) = stack.pop() {
            if let Some(next) = item.iter.pop() {
                let index = item.index;

                // push current item back to the stack
                stack.push(item);

                // ignore any back edges to guarantee acyclicity
                if next <= index {
                    continue;
                }
                // ignore already visited (and finalized) nodes
                if was_visited[next as usize] {
                    continue;
                }

                if is_in_stack[next as usize] {
                    panic!("cycle detected, even though it should have been removed")
                }
                is_in_stack[next as usize] = true;

                stack.push(BacktrackStackItem {
                    index: next,
                    iter: get_instr_out_edges(superset, superset_index, next as usize),
                });
            } else {
                is_in_stack[item.index as usize] = false;
                if !was_visited[item.index as usize] {
                    was_visited[item.index as usize] = true;
                    result.push(item.index);
                } else {
                    panic!("We have left the node twice?? (index: {})", item.index)
                }
            }
        }
    }

    result.reverse();

    result
}

// walk all simple paths using recursion (TODO: can this fail because of too much recursion?)
fn walk_data_dep(
    graph: &mut Graph,
    superset: &[(Address32, InstructionFeature, Option<Label>)],
    superset_index: &FxHashMap<Address32, Index32>,
) {
    fn collect_edges(
        graph: &mut Graph,
        superset: &[(Address32, InstructionFeature, Option<Label>)],
        index: usize,
        data_state: &DataDepState,
    ) {
        let (_, instr, _) = superset[index];
        for used_reg in instr.uses.iter() {
            let define_indices = &data_state[used_reg];
            for define_index in define_indices.iter() {
                graph.add_edge(
                    index as Index32,
                    define_index as Index32,
                    RelationType::DataDependency,
                );
                graph.add_edge(
                    define_index as Index32,
                    index as Index32,
                    RelationType::DataDependent,
                );
            }
        }
    }

    fn apply_state(
        superset: &[(Address32, InstructionFeature, Option<Label>)],
        index: usize,
        data_state: &mut DataDepState,
    ) {
        let (_, instr, _) = superset[index];
        for defined_reg in instr.defines.iter() {
            data_state[defined_reg] = SingleVec::from_single(index as Index32);
        }
    }

    fn aggregate_state(data_state: &DataDepState, dst_state: &mut DataDepState) {
        for (src, dst) in data_state.values().zip(dst_state.values_mut()) {
            for src in src.iter() {
                if !dst.contains(src) {
                    dst.push(src);
                }
            }
        }
    }

    let topo_order = toposort(superset, superset_index);

    let len = superset.len();
    let mut states = vec![DataDepState::default(); len];

    // walk the graph in topological order, collecting edges and updating the data dependency state
    for index in topo_order {
        let state = &states[index as usize];
        collect_edges(graph, superset, index as usize, state);
        let mut state = state.clone();
        // dbg!(index);
        // dbg!(&state);
        // dbg!(superset[index as usize].1);
        apply_state(superset, index as usize, &mut state);
        // dbg!(&state);
        for succ in get_instr_out_edges(superset, superset_index, index as usize) {
            aggregate_state(&state, &mut states[succ as usize]);
        }
        // we will never need this instr again, so we can clear the state
        states[index as usize].clear();
    }
}

#[derive(Serialize, Deserialize)]
pub struct Graph {
    pub edges: Vec<(Index32, Index32)>,
    pub edge_types: Vec<RelationType>,
}

impl Graph {
    pub fn new() -> Self {
        Self {
            edges: Vec::new(),
            edge_types: Vec::new(),
        }
    }

    pub fn add_edge(&mut self, from: Index32, to: Index32, edge_type: RelationType) {
        self.edges.push((from, to));
        self.edge_types.push(edge_type);
    }

    pub fn sort(&mut self) {
        let mut perm = permutation::sort(&self.edges);

        perm.apply_slice_in_place(&mut self.edges);
        perm.apply_slice_in_place(&mut self.edge_types);
    }
}

#[derive(Serialize, Deserialize)]
pub struct GraphSample {
    // we store out superset disassembly, but we don't need the addresses
    pub superset: Vec<(InstructionFeature, Option<Label>)>,
    // stores the graph, using indices into superset
    pub graph: Graph,
    pub source: Option<String>,
}

impl GraphSample {
    pub fn new(superset: SupersetSample) -> Self {
        assert!(superset.superset.len() < i32::MAX as usize);

        let mut graph = Graph::new();

        // TODO: we can devise a custom collection to map addresses to something
        // it can be implemented as a vector (or a group of them?), as the addresses are usually densely packed in some range
        let mut index = FxHashMap::default();
        for (i, &(addr, _, _)) in superset.superset.iter().enumerate() {
            index.insert(addr as Address32, i as Index32);
        }

        walk_data_dep(&mut graph, &superset.superset, &index);

        for (i, &(addr, ref instr, _)) in superset.superset.iter().enumerate() {
            let i = i as Index32;
            if instr.falls_through {
                let next_addr = addr + instr.size as u32;
                if let Some(next) = index.get(&next_addr).cloned() {
                    graph.add_edge(i, next, RelationType::Next);
                    graph.add_edge(next, i, RelationType::Previous);
                }

                for j in addr..next_addr {
                    if let Some(overlap) = index.get(&j).cloned() {
                        graph.add_edge(i, overlap, RelationType::Overlap);
                        graph.add_edge(overlap, i, RelationType::Overlap);
                    }
                }
            }

            if let Some(target) = instr.jump_target {
                if let Some(jump) = index.get(&target).cloned() {
                    // dbg!((addr, target, i, jump));
                    graph.add_edge(i, jump, RelationType::JumpTo);
                    graph.add_edge(jump, i, RelationType::JumpFrom);
                }
            }
        }

        graph.sort();

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

        let instruction_sizes = Array1::from_iter(self.superset.iter().map(
            |(i, _)| i.size - 1, /* substraction is to make it 0-indexed class index */
        ));
        let instruction_codes =
            Array1::from_iter(self.superset.iter().map(|(i, _)| vocab[i.code] as i32));
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
        let relation_types =
            Array1::from_iter(self.graph.edge_types.into_iter().map(|t| u8::from(t)));
        let relations = Array2::from_shape_vec(
            (self.graph.edges.len(), 2),
            self.graph
                .edges
                .into_iter()
                .flat_map(|(a, b)| [a as i32, b as i32])
                .collect(),
        )
        .unwrap();

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
