mod debian;
pub use debian::DebianSourceInfo;

use crate::model::ExecutableSample;

use anyhow::{Context, Result};
use futures_util::{pin_mut, Stream};
use serde::Deserialize;
use tracing::info;

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "type")]
#[serde(rename_all = "kebab-case")]
pub enum SpecificSourceInfo {
    Debian(DebianSourceInfo),
}

#[derive(Deserialize, Clone, Debug)]
pub struct SourceInfo {
    pub subdirectory: String,
    #[serde(flatten)]
    pub specific: SpecificSourceInfo,
}

pub fn fetch_source(
    source_info: &SourceInfo,
) -> impl Stream<Item = Result<(String, ExecutableSample)>> + '_ {
    use futures_util::StreamExt;

    let stream = match &source_info.specific {
        SpecificSourceInfo::Debian(debian) => debian::fetch_debian(debian),
    };

    stream
        .map(|r| r.map(|(name, sample)| (format!("{}/{}", source_info.subdirectory, name), sample)))
}

pub async fn fetch_source_to_directory(
    source_info: &SourceInfo,
    directory: &std::path::Path,
) -> Result<()> {
    use futures_util::StreamExt;

    let stream = fetch_source(source_info);
    pin_mut!(stream);

    while let Some(r) = stream.next().await {
        let (name, sample) =
            r.with_context(|| format!("failed while fetching {}", source_info.subdirectory))?;
        let path = directory.join(name);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .with_context(|| format!("failed to create directory {}", path.display()))?;

        let mut file = std::fs::File::create(&path)
            .with_context(|| format!("failed to create file {}", path.display()))?;

        sample
            .serialize_into(&mut file)
            .with_context(|| format!("failed to serialize {}", path.display()))?;
    }

    Ok(())
}

pub async fn fetch_sources_to_directory(
    source_infos: &[SourceInfo],
    directory: &std::path::Path,
) -> Result<()> {
    tokio::fs::create_dir_all(directory).await?;

    for source in source_infos {
        info!("fetching {}...", source.subdirectory);
        fetch_source_to_directory(source, directory).await?;
    }

    Ok(())
}
