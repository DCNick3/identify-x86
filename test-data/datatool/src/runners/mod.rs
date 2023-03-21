mod ida;

use anyhow::Result;
use serde::Deserialize;
use std::collections::BTreeSet;
use strum::EnumString;

#[derive(Deserialize)]
pub struct DisasmToolConfig {
    pub ida: IdaConfig,
}

#[derive(Debug, Copy, Clone, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub enum DisasmTool {
    Ida,
}

pub async fn run_tool(
    runner: DisasmTool,
    config: &DisasmToolConfig,
    sample: &ExecutableSample,
) -> Result<BTreeSet<u32>> {
    match runner {
        DisasmTool::Ida => ida::run_ida(&config.ida, sample).await,
    }
}

use crate::model::ExecutableSample;
pub use ida::IdaConfig;
