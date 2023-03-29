mod tool;

use crate::model::ExecutableSample;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub use tool::{DeepDiConfig, DisasmToolConfig, DisasmToolName, DisasmToolWithConfig, IdaConfig};

#[derive(Serialize, Deserialize)]
pub struct DisassemblyResult {
    pub predicted_instructions: BTreeSet<u32>,
}

#[async_trait]
pub trait ExecutableDisassembler {
    async fn disassemble(&self, sample: &ExecutableSample) -> Result<DisassemblyResult>;
}
