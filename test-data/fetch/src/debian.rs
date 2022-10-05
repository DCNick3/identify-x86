use crate::model::ExecutableSample;
use crate::Interval;
use anyhow::{anyhow, bail, Context, Result};
use async_tar::{Archive, Entry, EntryType};
use debian_packaging::deb::reader::{BinaryPackageEntry, BinaryPackageReader};
use debian_packaging::repository::{BinaryPackageFetch, ReleaseReader};
use futures_util::{AsyncRead, AsyncReadExt, StreamExt};
use object::read::elf::ElfFile32;
use object::{Architecture, Object, ObjectSection};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::io::Read;
use std::mem;
use std::sync::Arc;
use tracing::{info, warn};
use yoke::Yokeable;

pub struct DebianConfig {
    pub mirror: String,
    /// Debug mirror URL for debian 9+
    /// should no None for older versions
    /// See [https://wiki.debian.org/AutomaticDebugPackages]
    pub debug_mirror: Option<String>,
    pub distribution: String,
    pub arch: String,
}

impl Default for DebianConfig {
    fn default() -> Self {
        Self {
            mirror: "http://deb.debian.org/debian".to_string(),
            debug_mirror: Some("http://debug.mirrors.debian.org/debian-debug".to_string()),
            distribution: "buster".to_string(),
            arch: "i386".to_string(),
        }
    }
}

async fn find_packages<'a>(
    config: &'a DebianConfig,
    release_reader: &'a dyn ReleaseReader,
    names: Arc<HashSet<String>>,
) -> Result<BTreeMap<String, BinaryPackageFetch<'a>>> {
    let arch = config.arch.clone();
    let names_copy = names.clone();
    let packages = release_reader
        .resolve_package_fetches(
            Box::new(move |packages| !packages.is_installer && packages.architecture == arch),
            Box::new(move |package| names.contains(package.package().unwrap())),
            8,
        )
        .await
        .context("Getting package index")?;

    let res: BTreeMap<_, _> = packages
        .into_iter()
        .map(|package| (package.control_file.package().unwrap().to_string(), package))
        .collect();

    for name in names_copy.iter() {
        if !res.contains_key(name) {
            bail!("Package {} not found", name);
        }
    }

    Ok(res)
}

enum DebugPackageSource {
    NormalRepo,
    DebugRepo,
}

async fn find_debug_packages<'a>(
    config: &DebianConfig,
    release_reader: &'a dyn ReleaseReader,
    debug_release_reader: Option<&'a dyn ReleaseReader>,
    names: Arc<HashSet<String>>,
) -> Result<BTreeMap<String, (DebugPackageSource, BinaryPackageFetch<'a>)>> {
    // debian is kinda a mess: some packages are created manually
    // they have a -dbg suffix and are located in the main repo
    // others are generated automagically and have a -dbgsym suffix

    let arch = config.arch.clone();
    let names_copy = names.clone();
    let packages = release_reader
        .resolve_package_fetches(
            Box::new(move |packages| !packages.is_installer && packages.architecture == arch),
            Box::new(move |package| {
                package
                    .package()
                    .unwrap()
                    .strip_suffix("-dbg")
                    .map(|name| names.contains(name))
                    .unwrap_or(false)
            }),
            8,
        )
        .await
        .context("Getting package index")?;

    let arch = config.arch.clone();
    let names = names_copy.clone();
    let debug_packages = match debug_release_reader.as_ref() {
        Some(debug_release_reader) => debug_release_reader
            .resolve_package_fetches(
                Box::new(move |packages| !packages.is_installer && packages.architecture == arch),
                Box::new(move |package| {
                    package
                        .package()
                        .unwrap()
                        .strip_suffix("-dbgsym")
                        .map(|name| names.contains(name))
                        .unwrap_or(false)
                }),
                8,
            )
            .await
            .context("Getting package index")?,
        None => vec![],
    };

    let res: BTreeMap<_, _> = packages
        .into_iter()
        .map(|package| {
            (
                package
                    .control_file
                    .package()
                    .unwrap()
                    .strip_suffix("-dbg")
                    .unwrap()
                    .to_string(),
                (DebugPackageSource::NormalRepo, package),
            )
        })
        .chain(debug_packages.into_iter().map(|package| {
            (
                package
                    .control_file
                    .package()
                    .unwrap()
                    .strip_suffix("-dbgsym")
                    .unwrap()
                    .to_string(),
                (DebugPackageSource::DebugRepo, package),
            )
        }))
        .collect();

    for name in names_copy.iter() {
        // __sometimes__ old debian packages contain entire new builds of the same package
        // in this case it will be specified in required package names and we would want to install it
        if name.ends_with("-dbg") {
            continue;
        }
        if !res.contains_key(name) {
            bail!("Debug package {} not found", name);
        }
    }

    Ok(res)
}

async fn extract_files<
    R: Read,
    T,
    Ft: Future<Output = Result<Option<T>>>,
    F: Fn(Entry<Archive<Box<dyn AsyncRead + Unpin>>>) -> Ft,
>(
    mut reader: BinaryPackageReader<R>,
    map_filter: F,
) -> Result<BTreeMap<String, T>> {
    while let Some(entry) = reader.next_entry() {
        let entry = entry.context("Reading a package entry")?;
        match entry {
            BinaryPackageEntry::DebianBinary(_) | BinaryPackageEntry::Control(_) => {}
            BinaryPackageEntry::Data(data) => {
                let mut res = BTreeMap::new();

                let mut entries = data
                    .into_inner()
                    .entries()
                    .context("Reading package data entries")?;
                while let Some(entry) = entries.next().await {
                    let entry = entry.context("Reading package data entry")?;
                    if matches!(entry.header().entry_type(), EntryType::Regular) {
                        let path = entry.path().unwrap().to_str().unwrap().to_string();
                        if let Some(transformed_entry) = map_filter(entry)
                            .await
                            .with_context(|| format!("Processing entry {}", path))?
                        {
                            res.insert(path, transformed_entry);
                        }
                    }
                }
                return Ok(res);
            }
        }
    }

    bail!("Missing a data entry")
}

// #[derive(Yokeable)]
#[repr(transparent)]
struct YokableElf<'a>(ElfFile32<'a>);

unsafe impl<'a> Yokeable<'a> for YokableElf<'static> {
    type Output = ElfFile32<'a>;

    fn transform(&'a self) -> &'a Self::Output {
        &self.0
    }

    fn transform_owned(self) -> Self::Output {
        self.0
    }

    unsafe fn make(from: Self::Output) -> Self {
        // We're just doing mem::transmute() here, however Rust is
        // not smart enough to realize that Bar<'a> and Bar<'static> are of
        // the same size, so instead we use transmute_copy

        // This assert will be optimized out, but is included for additional
        // peace of mind as we are using transmute_copy
        debug_assert!(mem::size_of::<Self::Output>() == mem::size_of::<Self>());
        let ptr: *const Self = (&from as *const Self::Output).cast();
        mem::forget(from);
        std::ptr::read(ptr)
    }

    fn transform_mut<F>(&'a mut self, f: F)
    where
        F: 'static + for<'b> FnOnce(&'b mut Self::Output),
    {
        unsafe { f(mem::transmute::<&mut Self, &mut Self::Output>(self)) }
    }
}

type YokeElf = yoke::Yoke<YokableElf<'static>, Arc<[u8]>>;

async fn map_filter_exec(
    mut entry: Entry<Archive<Box<dyn AsyncRead + Unpin>>>,
) -> Result<Option<YokeElf>> {
    Ok({
        let mut buffer = Vec::new();
        entry
            .read_to_end(&mut buffer)
            .await
            .context("Reading the executable file")?;

        let buffer: Arc<[u8]> = Arc::from(buffer.as_ref());

        if let Ok(elf) = YokeElf::try_attach_to_cart(buffer, |cart| ElfFile32::parse(cart)) {
            if elf.get().architecture() == Architecture::I386
                && elf.get().build_id().unwrap().is_some()
            {
                Some(elf)
            } else {
                None
            }
        } else {
            None
        }
    })
}

static DEBUG_FILENAME_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"^./usr/lib/debug/.build-id/([0-9a-f]{2})/([0-9a-f]{38}).debug$"#).unwrap()
});

async fn map_filter_debug(
    mut entry: Entry<Archive<Box<dyn AsyncRead + Unpin>>>,
) -> Result<Option<(String, YokeElf)>> {
    Ok({
        let filename = entry.path().unwrap().to_str().unwrap().to_string();
        if let Some(c) = DEBUG_FILENAME_REGEX.captures(&filename) {
            let buildid = format!(
                "{}{}",
                c.get(1).unwrap().as_str(),
                c.get(2).unwrap().as_str()
            );

            let mut buffer = Vec::new();
            entry
                .read_to_end(&mut buffer)
                .await
                .context("Reading the executable file")?;

            let buffer: Arc<[u8]> = Arc::from(buffer.as_ref());

            if let Ok(elf) = YokeElf::try_attach_to_cart(buffer, |cart| ElfFile32::parse(cart)) {
                if elf.get().architecture() == Architecture::I386 {
                    Some((buildid, elf))
                } else {
                    None
                }
            } else {
                // println!("Could not parse debug info file {}", filename);
                None
            }
        } else {
            None
        }
    })
}

pub async fn dump_debian<S, F, P, I>(
    config: &DebianConfig,
    package_names: P,
    save_sample: S,
) -> Result<()>
where
    P: IntoIterator<Item = I>,
    I: AsRef<str>,
    S: Fn(&str, &str, ExecutableSample) -> F,
    F: Future<Output = Result<()>>,
{
    let packages = Arc::new(
        package_names
            .into_iter()
            .map(|v| v.as_ref().to_string())
            .collect::<HashSet<_>>(),
    );

    info!("Getting debian packages {:?}", packages);
    let repo_reader = debian_packaging::repository::reader_from_str(&config.mirror)
        .context("Getting a RepositoryRootReader")?;

    let debug_repo_reader = config
        .debug_mirror
        .as_ref()
        .map(|mirror| {
            debian_packaging::repository::reader_from_str(mirror)
                .context("Getting a RepositoryRootReader for debug packages")
        })
        .transpose()?;

    let release_reader = repo_reader
        .release_reader(&config.distribution)
        .await
        .context("Getting a ReleaseReader")?;

    let debug_release_reader = match debug_repo_reader.as_ref() {
        Some(debug_repo_reader) => Some(
            debug_repo_reader
                .release_reader(&format!("{}-debug", &config.distribution))
                .await
                .context("Getting a ReleaseReader for debug packages")?,
        ),
        None => None,
    };

    let time = std::time::Instant::now();

    info!("Fetching package indices...");
    let packages_to_fetch = find_packages(config, release_reader.as_ref(), packages.clone())
        .await
        .context("Finding packages")?;
    let debug_packages_to_fetch = find_debug_packages(
        config,
        release_reader.as_ref(),
        debug_release_reader.as_ref().map(|v| v.as_ref()),
        packages.clone(),
    )
    .await
    .context("Finding debug packages")?;

    let elapsed = time.elapsed();

    info!(
        "Found {} packages to fetch in {}s",
        packages_to_fetch.len(),
        elapsed.as_secs()
    );

    // let total_size: u64 = packages_to_fetch
    //     .values()
    //     .map(|v| v.size)
    //     .chain(debug_packages_to_fetch.values().map(|v| v.1.size))
    //     .sum();

    // let progress = ProgressBar::new(total_size).with_style(
    //     ProgressStyle::with_template(
    //         "[{elapsed_precise}] {bar:40.cyan/blue} {bytes:>7}/{total_bytes:7} [{bytes_per_sec:5}] {msg}",
    //     )
    //         .unwrap(),
    // );

    for package_name in packages_to_fetch.keys() {
        // progress.println(format!("[[{}]]", package_name));
        let package = packages_to_fetch.get(package_name).unwrap();
        let debug_package = debug_packages_to_fetch.get(package_name);

        // let package_size = package.size;
        // let debug_package_size = debug_package.size;

        // progress.set_message(format!("Fetching {}", package_name));
        let package = repo_reader
            .fetch_binary_package_deb_reader(package.clone())
            .await
            .with_context(|| format!("Downloading package {}", package_name))?;
        // progress.inc(package_size);

        // progress.set_message(format!("Fetching debug symbols for {}", package_name));
        let debug_package = match debug_package {
            Some((debug_package_source, debug_package)) => {
                let debug_package_rr = match debug_package_source {
                    DebugPackageSource::NormalRepo => &repo_reader,
                    DebugPackageSource::DebugRepo => debug_repo_reader.as_ref().unwrap(),
                };
                Some(
                    debug_package_rr
                        .fetch_binary_package_deb_reader(debug_package.clone())
                        .await
                        .with_context(|| format!("Downloading debug package {}", package_name))?,
                )
            }
            None => None,
        };

        // progress.inc(debug_package_size);

        let executables = extract_files(package, map_filter_exec)
            .await
            .with_context(|| format!("Extracting package {}", package_name))?;

        let debugs = match debug_package {
            Some(debug_package) => extract_files(debug_package, map_filter_debug)
                .await
                .with_context(|| format!("Extracting debug package {}", package_name))?
                .into_values()
                .collect::<BTreeMap<_, _>>(),
            None => BTreeMap::new(),
        };

        for (filename, executable) in executables {
            let build_id = hex::encode(executable.get().build_id().unwrap().unwrap());
            let debug_info = debugs.get(&build_id);
            info!("EXE {} {}", build_id, filename);

            let sample = ExecutableSample::from_debian(
                package_name,
                &filename,
                executable.get(),
                debug_info.map(|v| v.get()),
            )
            .with_context(|| {
                format!(
                    "Parsing executable {} in package {}",
                    filename, package_name
                )
            })?;

            // executable.

            // compute .text section coverage to filter out executables that have incomplete debug info
            // for gcc-compiled linux binaries we expect > 95% coverage
            let (covered, total) = {
                use object::ObjectSection;
                let text_region = executable
                    .get()
                    .section_by_name(".text")
                    .ok_or_else(|| anyhow!("No .text section"))?;

                let address = text_region.address().try_into().unwrap();
                let size = text_region.size().try_into().unwrap();

                let mut classes = sample.classes.clone();
                classes.filter_to(Interval::from_start_and_len(address, size));
                (classes.coverage(), size)
            };

            if (covered as f64 / total as f64) > 0.75 {
                save_sample(package_name, &filename, sample)
                    .await
                    .context("Saving executable sample")?;
            } else {
                warn!(
                    "Executable {} in package {} (has_debug = {}) has low coverage ({}/{}). Skipping.",
                    filename,
                    package_name,
                    debug_info.is_some(),
                    covered,
                    total
                );
            }
        }

        // progress.println(format!(
        //     "executables: {:?}",
        //     executables.keys().collect::<Vec<_>>()
        // ));
    }

    Ok(())
}
