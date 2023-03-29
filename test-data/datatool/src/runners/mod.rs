mod deepdi;
mod ida;

use crate::model::ExecutableSample;
use anyhow::Result;
use serde::Deserialize;
use std::collections::BTreeSet;
use strum::EnumString;

pub use deepdi::DeepDiConfig;
pub use ida::IdaConfig;

#[derive(Deserialize)]
pub struct DisasmToolConfig {
    pub ida: IdaConfig,
    pub deepdi: DeepDiConfig,
}

#[derive(Debug, Copy, Clone, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub enum DisasmTool {
    Ida,
    Deepdi,
}

pub async fn run_tool(
    runner: DisasmTool,
    config: &DisasmToolConfig,
    sample: &ExecutableSample,
) -> Result<BTreeSet<u32>> {
    match runner {
        DisasmTool::Ida => ida::run_ida(&config.ida, sample).await,
        DisasmTool::Deepdi => deepdi::run_deepdi(&config.deepdi, sample).await,
    }
}
