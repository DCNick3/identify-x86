mod byteweight;
mod debian;

pub use byteweight::ByteweightSourceInfo;
pub use debian::DebianSourceInfo;

use crate::model::ExecutableSample;

use anyhow::{Context, Result};
use futures_util::{pin_mut, Stream};
use serde::{Deserialize, Serialize};
use tokio_util::either::Either;
use tracing::{debug, info};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type")]
#[serde(rename_all = "kebab-case")]
pub enum SpecificSourceInfo {
    Debian(DebianSourceInfo),
    Byteweight(ByteweightSourceInfo),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SourceInfo {
    pub subdirectory: String,
    #[serde(flatten)]
    pub specific: SpecificSourceInfo,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FetchConfig {
    pub sources: Vec<SourceInfo>,
}

pub fn fetch_source(
    source_info: &SourceInfo,
) -> impl Stream<Item = Result<(String, ExecutableSample)>> + '_ {
    use futures_util::StreamExt;

    let stream = match &source_info.specific {
        SpecificSourceInfo::Debian(debian) => Either::Left(debian::fetch_debian(debian)),
        SpecificSourceInfo::Byteweight(byteweight) => {
            Either::Right(byteweight::fetch_byteweight(byteweight))
        }
    };

    stream
        .map(|r| r.map(|(name, sample)| (format!("{}/{}", source_info.subdirectory, name), sample)))
}

pub async fn fetch_source_to_directory(
    source_info: &SourceInfo,
    directory: &std::path::Path,
) -> Result<()> {
    use futures_util::StreamExt;

    match tokio::fs::remove_dir_all(directory.join(&source_info.subdirectory)).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => anyhow::bail!("failed to remove directory {}: {}", directory.display(), e),
    }

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

fn read_stamp(path: &std::path::Path) -> Result<Option<SpecificSourceInfo>> {
    let stamp_path = path.join("sync-stamp");
    match std::fs::read_to_string(&stamp_path) {
        Ok(stamp) => {
            let stamped_config = serde_json::from_str::<SpecificSourceInfo>(&stamp)
                .with_context(|| format!("failed to parse {}", stamp_path.display()))?;
            Ok(Some(stamped_config))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => Err(e).with_context(|| format!("failed to read {}", stamp_path.display())),
    }
}

fn write_stamp(path: &std::path::Path, config: &SpecificSourceInfo) -> Result<()> {
    let stamp_path = path.join("sync-stamp");
    let stamp = serde_json::to_string(config)
        .with_context(|| format!("failed to serialize {}", stamp_path.display()))?;
    std::fs::write(&stamp_path, stamp)
        .with_context(|| format!("failed to write {}", stamp_path.display()))?;
    Ok(())
}

pub async fn sync_sources_to_directory(
    fetch_config: &FetchConfig,
    directory: &std::path::Path,
) -> Result<()> {
    tokio::fs::create_dir_all(directory).await?;

    // ensure no sources have the same subdirectory
    {
        let mut subdirectories = std::collections::HashSet::new();
        for source in &fetch_config.sources {
            if !subdirectories.insert(&source.subdirectory) {
                anyhow::bail!(
                    "subdirectory {} is used for multiple sources",
                    source.subdirectory
                );
            }
        }
    }

    // find which sources are outdated or missing
    let mut outdated = Vec::new();
    for source in &fetch_config.sources {
        match read_stamp(&directory.join(&source.subdirectory))? {
            Some(stamped_config) if stamped_config == source.specific => {
                debug!("{} is up to date", source.subdirectory);
            }
            _ => {
                debug!("{} is outdated", source.subdirectory);
                outdated.push(source);
            }
        }
    }

    info!("{} sources are outdated", outdated.len());

    for source in outdated {
        info!("fetching {}...", source.subdirectory);
        fetch_source_to_directory(source, directory)
            .await
            .with_context(|| format!("Fetching source {}", source.subdirectory))?;
        write_stamp(&directory.join(&source.subdirectory), &source.specific)
            .with_context(|| format!("Failed to write stamp for source {}", source.subdirectory))?;
    }

    Ok(())
}
