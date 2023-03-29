mod deepdi;
mod ida;

use crate::model::ExecutableSample;
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use strum::EnumString;

use crate::disassembly::{DisassemblyResult, ExecutableDisassembler};
pub use deepdi::DeepDiConfig;
pub use ida::IdaConfig;

#[derive(Deserialize, Clone)]
pub struct DisasmToolConfig {
    pub ida: IdaConfig,
    pub deepdi: DeepDiConfig,
}

#[derive(Debug, Copy, Clone, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub enum DisasmToolName {
    Ida,
    Deepdi,
}

impl DisasmToolName {
    pub fn with_config(&self, config: &DisasmToolConfig) -> DisasmToolWithConfig {
        match self {
            DisasmToolName::Ida => DisasmToolWithConfig::Ida(config.ida.clone()),
            DisasmToolName::Deepdi => DisasmToolWithConfig::Deepdi(config.deepdi.clone()),
        }
    }
}

pub enum DisasmToolWithConfig {
    Ida(IdaConfig),
    Deepdi(DeepDiConfig),
}

#[async_trait]
impl ExecutableDisassembler for DisasmToolWithConfig {
    async fn disassemble(&self, sample: &ExecutableSample) -> Result<DisassemblyResult> {
        match self {
            DisasmToolWithConfig::Ida(config) => ida::run_ida(&config, sample).await,
            DisasmToolWithConfig::Deepdi(config) => deepdi::run_deepdi(&config, sample).await,
        }
    }
}
