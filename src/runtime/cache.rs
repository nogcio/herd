//! Read-only artifact resolution in the pinned native HF cache, never a registry.
use crate::{
    config::{Presets, Settings, read_bounded},
    downloads::{Plan, validate_repo},
};
use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Component, Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostCache {
    configuration: String,
    root: PathBuf,
    automatic_configs: Vec<PathBuf>,
}

fn configuration_identity(settings: &Settings) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(settings)?)))
}

/// Match b11856 hf-cache.cpp, then pass this identity explicitly to the host.
pub(crate) fn configured_root() -> Result<PathBuf> {
    for (key, suffix) in [
        ("LLAMA_CACHE", ""),
        ("HF_HUB_CACHE", ""),
        ("HUGGINGFACE_HUB_CACHE", ""),
        ("HF_HOME", "hub"),
        ("XDG_CACHE_HOME", "huggingface/hub"),
        ("HOME", ".cache/huggingface/hub"),
    ] {
        if let Some(value) = std::env::var_os(key).filter(|v| !v.is_empty()) {
            let path = PathBuf::from(value).join(suffix);
            ensure!(
                path.is_absolute(),
                "Native cache directory must be absolute"
            );
            return Ok(path);
        }
    }
    dirs::home_dir()
        .map(|p| p.join(".cache/huggingface/hub"))
        .context("Cannot locate the native cache directory")
}

/// Private host identity survives backend restarts; no model catalogue is stored.
pub(crate) fn record_host(
    settings: &Settings,
    root: &Path,
    automatic_configs: &[PathBuf],
) -> Result<()> {
    ensure!(
        root.is_absolute(),
        "Native cache directory must be absolute"
    );
    let bytes = serde_json::to_vec(&HostCache {
        configuration: configuration_identity(settings)?,
        root: root.into(),
        automatic_configs: automatic_configs.into(),
    })?;
    let mut file = tempfile::NamedTempFile::new_in(&settings.state_dir)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(settings.state_dir.join("cache-root.json"))
        .map_err(|_| anyhow!("Cannot record native host cache identity"))?;
    Ok(())
}

pub(crate) struct Cache {
    root: PathBuf,
    automatic_configs: Vec<PathBuf>,
}
impl Cache {
    pub(crate) fn validate_root(root: &Path) -> Result<()> {
        #[derive(Deserialize)]
        struct Journal {
            downloads: Vec<Transfer>,
        }
        #[derive(Deserialize)]
        struct Transfer {
            model: String,
            #[serde(default)]
            partials: Vec<Partial>,
        }
        #[derive(Deserialize)]
        struct Partial {
            path: PathBuf,
        }
        let journal = root.join("unfinished-downloads.json");
        match std::fs::symlink_metadata(&journal) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
            Ok(_) => {}
        }
        let data: Journal = serde_json::from_str(&read_bounded(&journal)?).map_err(|_| {
            anyhow!("Invalid native unfinished-download journal; cache mutation refused")
        })?;
        ensure!(
            data.downloads.len() <= 128,
            "Native unfinished-download journal exceeds entry limit"
        );
        let root = std::fs::canonicalize(root)?;
        for transfer in data.downloads {
            let repo = transfer.model.split(':').next().unwrap_or("");
            validate_repo(repo)?;
            ensure!(
                transfer.partials.len() <= 32,
                "Native transfer exceeds partial-file limit"
            );
            let blobs = root
                .join(format!("models--{}", repo.replace('/', "--")))
                .join("blobs");
            for partial in transfer.partials {
                let name = partial
                    .path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                let hash = name.strip_suffix(".downloadInProgress").unwrap_or("");
                ensure!(
                    partial.path.is_absolute()
                        && [40, 64].contains(&hash.len())
                        && hash.bytes().all(|b| b.is_ascii_hexdigit()),
                    "Native partial path is outside the supported blob contract"
                );
                let parent = partial
                    .path
                    .parent()
                    .context("Native partial has no parent")?;
                ensure!(
                    std::fs::canonicalize(parent)? == blobs && blobs.starts_with(&root),
                    "Native partial unlink target escapes its repository"
                );
            }
        }
        Ok(())
    }
    pub(crate) fn for_host(settings: &Settings) -> Result<Self> {
        let text = read_bounded(&settings.state_dir.join("cache-root.json"))
            .map_err(|_| anyhow!("Native cache identity is unavailable; explicitly start the configured router before cache operations"))?;
        let cache: HostCache = serde_json::from_str(&text)
            .map_err(|_| anyhow!("Invalid native host cache identity"))?;
        ensure!(
            cache.configuration == configuration_identity(settings)?
                && cache.root.is_absolute()
                && cache.automatic_configs.len() == 2
                && cache.automatic_configs.iter().all(|p| p.is_absolute()),
            "Native cache identity does not match the configured router"
        );
        Self::validate_root(&cache.root)?;
        Ok(Self {
            root: cache.root,
            automatic_configs: cache.automatic_configs,
        })
    }
    pub(crate) fn automatic_configs(&self) -> &[PathBuf] {
        &self.automatic_configs
    }
    fn repo(&self, repo: &str) -> Result<PathBuf> {
        validate_repo(repo)?;
        Ok(self
            .root
            .join(format!("models--{}", repo.replace('/', "--"))))
    }
    // Native readers use refs/main and snapshots/<commit>. Restrict fallback
    // ambiguity rather than reproducing upstream's filesystem-order selection.
    fn files(&self, repo: &str) -> Result<Vec<(String, PathBuf)>> {
        let base = self.repo(repo)?;
        if !base.try_exists()? {
            return Ok(Vec::new());
        }
        let canonical = std::fs::canonicalize(&base)?;
        ensure!(
            canonical.starts_with(std::fs::canonicalize(&self.root)?),
            "Cache repository escapes native cache root"
        );
        let reference = match read_bounded(&base.join("refs/main")) {
            Ok(value) => value,
            Err(_) => return Err(anyhow!("Native cache has no readable main reference")),
        };
        let commit = reference.lines().next().unwrap_or("");
        ensure!(
            commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid native cache reference"
        );
        let snapshot = base.join("snapshots").join(commit);
        if !snapshot.try_exists()? {
            return Ok(Vec::new());
        }
        ensure!(
            std::fs::canonicalize(&snapshot)?.starts_with(&canonical),
            "Cache snapshot escapes its repository"
        );
        let mut files = Vec::new();
        let mut todo = vec![(snapshot.clone(), 0usize)];
        let mut visited = 0;
        while let Some((dir, depth)) = todo.pop() {
            ensure!(depth <= 16, "Native cache exceeds directory depth limit");
            for entry in std::fs::read_dir(dir)? {
                visited += 1;
                ensure!(
                    visited <= 4096,
                    "Native cache exceeds 4096-entry inspection limit"
                );
                let entry = entry?;
                let path = entry.path();
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    todo.push((path, depth + 1));
                    continue;
                }
                ensure!(
                    kind.is_file() || kind.is_symlink(),
                    "Native cache contains a non-regular artifact"
                );
                if kind.is_symlink() {
                    let target = std::fs::read_link(&path)?;
                    let target = if target.is_absolute() {
                        target
                    } else {
                        path.parent().unwrap().join(target)
                    };
                    let parent = target
                        .parent()
                        .context("Native cache link target has no parent")?;
                    ensure!(
                        std::fs::canonicalize(parent)?.starts_with(&canonical),
                        "Native cache link unlink target escapes its repository"
                    );
                    let mut normalized = PathBuf::new();
                    for part in target.components() {
                        match part {
                            Component::ParentDir => {
                                normalized.pop();
                            }
                            Component::CurDir => {}
                            part => normalized.push(part.as_os_str()),
                        }
                    }
                    ensure!(
                        normalized.starts_with(&canonical),
                        "Native cache link unlink target escapes its repository"
                    );
                }
                let resolved = std::fs::canonicalize(&path).map_err(|_| {
                    anyhow!("Cannot resolve native cached artifact; cache actions refused")
                })?;
                ensure!(
                    resolved.starts_with(&canonical) && resolved.is_file(),
                    "Native cache artifact escapes its repository or is not regular"
                );
                let name = path
                    .strip_prefix(&snapshot)?
                    .to_str()
                    .context("Native cache filename must be UTF-8")?
                    .to_owned();
                files.push((name, path));
            }
        }
        Ok(files)
    }

    pub(crate) fn resolve(
        &self,
        model: &str,
        filename: Option<&str>,
        reviewed: Option<&Plan>,
    ) -> Result<PathBuf> {
        let (repo, tag) = model.split_once(':').unwrap_or((model, ""));
        ensure!(
            filename.is_some() || tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "Implicit native cache tags must be literal alphanumeric identifiers; use hf-file for other tags"
        );
        let files = self.files(repo)?;
        ensure!(
            !files.iter().any(|(n, _)| n == "preset.ini"),
            "Cached remote presets are not supported for loading"
        );
        let candidates: Vec<_> = files
            .iter()
            .filter(|(name, _)| {
                let upper = name.to_ascii_uppercase();
                name.ends_with(".gguf")
                    && (tag.is_empty()
                        || upper.contains(&format!("{}.", tag.to_ascii_uppercase()))
                        || upper.contains(&format!("{}-", tag.to_ascii_uppercase())))
            })
            .collect();
        let path = if let Some(filename) = filename {
            ensure!(
                !filename.is_empty()
                    && Path::new(filename)
                        .components()
                        .all(|p| matches!(p, Component::Normal(_))),
                "Invalid native cache filename"
            );
            files
                .iter()
                .find(|(name, _)| name == filename)
                .map(|(_, p)| p.clone())
        } else {
            ensure!(
                candidates.len() <= 1,
                "Native cached model selection is ambiguous; use an explicit native hf-file"
            );
            candidates.first().map(|(_, path)| path.clone())
        }
        .context("Local weights are missing; loading never downloads implicitly")?;
        if let Some(plan) = reviewed {
            ensure!(
                plan.repo == repo
                    && files
                        .iter()
                        .any(|(name, p)| name == &plan.filename && p == &path),
                "Cached artifact differs from the reviewed acquisition"
            );
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        ensure!(
            !name.contains("-of-")
                && !["mmproj", "imatrix", "mtp", "eagle", "dflash", "dspark"]
                    .iter()
                    .any(|s| name.contains(s)),
            "Split models and sidecar selection are not supported"
        );
        Ok(path)
    }

    pub(crate) fn ensure_unreferenced(&self, model: &str, presets: &Presets) -> Result<()> {
        let repo = model.split(':').next().unwrap_or(model);
        let base = self.repo(repo)?;
        // Inspect the same bounded native snapshot used by deletion, rejecting
        // escaping symlinks before the runtime can unlink their blob targets.
        let _ = self.files(repo)?;
        let canonical = std::fs::canonicalize(&base).ok();
        for profile in &presets.models {
            for key in ["model", "mmproj"] {
                if let Some(path) = profile.settings.get(key) {
                    ensure!(
                        !Path::new(path).starts_with(&base),
                        "A native preset references this cached repository"
                    );
                    let resolved = std::fs::canonicalize(path)
                        .map_err(|_| anyhow!("Cannot establish native preset artifact identity; cached files were not deleted"))?;
                    ensure!(
                        !canonical
                            .as_ref()
                            .is_some_and(|root| resolved.starts_with(root)),
                        "A native preset references this cached repository"
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_snapshot_resolution_and_reference_protection_are_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache {
            root: dir.path().join("hub"),
            automatic_configs: vec![],
        };
        let base = cache.root.join("models--example--tiny");
        let snapshot = base.join("snapshots").join("a".repeat(40));
        std::fs::create_dir_all(&snapshot).unwrap();
        std::fs::create_dir_all(base.join("refs")).unwrap();
        std::fs::create_dir_all(base.join("blobs")).unwrap();
        std::fs::write(base.join("refs/main"), "a".repeat(40)).unwrap();
        let blob = base.join("blobs/weights");
        std::fs::write(&blob, b"GGUFfixture").unwrap();
        std::os::unix::fs::symlink("../../blobs/weights", snapshot.join("tiny-Q8_0.gguf")).unwrap();
        let path = cache.resolve("example/tiny:Q8_0", None, None).unwrap();
        assert_eq!(std::fs::canonicalize(path).unwrap(), blob);
        assert!(cache.resolve("example/tiny:Q8.0", None, None).is_err());
        assert_eq!(
            cache
                .resolve("example/tiny:Q8.0", Some("tiny-Q8_0.gguf"), None)
                .unwrap(),
            snapshot.join("tiny-Q8_0.gguf")
        );
        let ini = dir.path().join("models.ini");
        std::fs::write(
            &ini,
            format!("version = 1\n[local]\nmodel = {}\n", blob.display()),
        )
        .unwrap();
        let presets = Presets::read(&ini).unwrap();
        assert!(
            cache
                .ensure_unreferenced("example/tiny:Q8_0", &presets)
                .is_err()
        );
        assert_eq!(std::fs::read(blob).unwrap(), b"GGUFfixture");
        assert_eq!(Presets::read(&ini).unwrap().revision, presets.revision);
        let partial = base
            .join("blobs")
            .join(format!("{}.downloadInProgress", "b".repeat(64)));
        std::fs::write(&partial, b"partial bytes").unwrap();
        std::fs::write(
            cache.root.join("unfinished-downloads.json"),
            serde_json::to_vec(&serde_json::json!({"downloads":[{
                "model":"example/tiny:Q8_0","sidecars":[],"partials":[{"kind":"","path":partial}]
            }]}))
            .unwrap(),
        )
        .unwrap();
        Cache::validate_root(&cache.root).unwrap();
        assert_eq!(std::fs::read(partial).unwrap(), b"partial bytes");
    }
}
