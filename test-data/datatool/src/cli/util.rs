use std::path::{Path, PathBuf};

pub fn collect_sample_paths(samples_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    walkdir::WalkDir::new(samples_path)
        .into_iter()
        .filter(|e| {
            e.as_ref()
                .map(|e| e.path().extension().unwrap_or_default() == "sample")
                .unwrap_or(false)
        })
        .map(|r| r.map(|e| e.into_path()).map_err(|e| e.into()))
        .collect::<anyhow::Result<Vec<PathBuf>>>()
}
