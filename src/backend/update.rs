//! One-shot, bounded state handoff for a coordinated management-only update.
use super::protocol::Report;
use crate::{config::Settings, regular_file};
use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, Metadata},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

const LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Default)]
struct BoundedBytes(Vec<u8>);

impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > LIMIT as usize {
            return Err(std::io::Error::other("Backend update state exceeds 4 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
}

impl Identity {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

pub(super) struct Executable {
    path: PathBuf,
    identity: Identity,
}

impl Executable {
    pub fn current() -> Result<Self> {
        let path = std::env::current_exe().context("Cannot identify the management executable")?;
        let metadata = fs::metadata("/proc/self/exe")
            .context("Cannot inspect the running management executable")?;
        Ok(Self {
            path,
            identity: Identity::from_metadata(&metadata),
        })
    }

    pub fn replacement(&self) -> Result<bool> {
        let file = regular_file::open(&self.path)
            .map_err(|_| anyhow!("Installed management executable is unavailable"))?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.permissions().mode() & 0o111 != 0,
            "Installed management executable is not executable"
        );
        let installed = Identity::from_metadata(&metadata);
        Ok(installed != self.identity)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Handoff {
    pub version: u32,
    pub preferences: Option<Settings>,
    pub report: Report,
}

impl Handoff {
    fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported backend update state format");
        if let Some(settings) = &self.preferences {
            settings.validate()?;
        }
        ensure!(
            self.report.operations.len() <= 64
                && self.report.operations.iter().all(|job| {
                    !job.id.is_empty()
                        && job.id.len() <= 128
                        && job.kind.len() <= 64
                        && matches!(job.status.as_str(), "succeeded" | "failed")
                        && (job.model.is_empty() || crate::config::validate_id(&job.model).is_ok())
                }),
            "Backend update state must contain only bounded terminal results"
        );
        Ok(())
    }
}

pub(super) fn path() -> Result<PathBuf> {
    Ok(Settings::path()?.with_file_name(".backend-update.json"))
}

pub(super) fn save(path: &Path, handoff: &Handoff) -> Result<()> {
    handoff.validate()?;
    let mut bytes = BoundedBytes::default();
    serde_json::to_writer(&mut bytes, handoff)
        .map_err(|_| anyhow!("Backend update state cannot be encoded within 4 MiB"))?;
    let parent = path
        .parent()
        .context("Backend update state has no parent")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&bytes.0)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|_| anyhow!("Backend update state already exists or cannot be saved"))?;
    Ok(())
}

pub(super) fn load(path: &Path) -> Result<Option<(Handoff, Vec<u8>)>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(anyhow!("Cannot inspect backend update state")),
        Ok(metadata) => ensure!(
            metadata.is_file() && metadata.permissions().mode() & 0o077 == 0,
            "Backend update state must be a private regular file"
        ),
    }
    let bytes = read(path)?;
    let handoff: Handoff =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("Invalid backend update state"))?;
    handoff.validate()?;
    Ok(Some((handoff, bytes)))
}

fn read(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let file = regular_file::open(path).map_err(|_| anyhow!("Cannot open backend update state"))?;
    ensure!(
        file.metadata()?.permissions().mode() & 0o077 == 0,
        "Backend update state must be private"
    );
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow!("Cannot read backend update state"))?;
    ensure!(
        bytes.len() as u64 <= LIMIT,
        "Backend update state exceeds 4 MiB"
    );
    Ok(bytes)
}

pub(super) fn consume(path: &Path, expected: &[u8]) -> Result<()> {
    ensure!(
        read(path)? == expected,
        "Backend update state changed during activation"
    );
    fs::remove_file(path).map_err(|_| anyhow!("Cannot consume backend update state"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::protocol::{ModelRecord, Operation};

    fn handoff() -> Handoff {
        Handoff {
            version: 1,
            preferences: None,
            report: Report {
                models: vec![ModelRecord {
                    id: "competitor".into(),
                    state: "ready".into(),
                    ..Default::default()
                }],
                operations: vec![Operation {
                    id: "previous-1".into(),
                    kind: "set".into(),
                    model: "example".into(),
                    status: "succeeded".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        }
    }

    #[test]
    fn executable_replacement_is_metadata_only_and_missing_or_nonexecutable_targets_fail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("service");
        fs::write(&path, b"not an executable script").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let executable = Executable {
            identity: Identity::from_metadata(&fs::metadata(&path).unwrap()),
            path: path.clone(),
        };
        assert!(!executable.replacement().unwrap());
        let replacement = dir.path().join("replacement");
        fs::write(&replacement, b"replacement bytes; never executed").unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o700)).unwrap();
        fs::rename(&replacement, &path).unwrap();
        assert!(executable.replacement().unwrap());
        assert_eq!(
            fs::read(&path).unwrap(),
            b"replacement bytes; never executed"
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(executable.replacement().is_err());
        fs::remove_file(&path).unwrap();
        assert!(executable.replacement().is_err());
    }

    #[test]
    fn handoff_retains_terminal_results_and_observations_without_replaying_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("handoff.json");
        let saved = handoff();
        save(&path, &saved).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
        let original = fs::read(&path).unwrap();
        assert!(save(&path, &saved).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        let (restored, token) = load(&path).unwrap().unwrap();
        assert_eq!(restored.report, saved.report);
        assert!(consume(&path, b"changed").is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        consume(&path, &token).unwrap();
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn invalid_or_in_progress_state_never_overwrites_an_existing_handoff() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("handoff.json");
        let mut saved = handoff();
        saved.version = 2;
        assert!(save(&path, &saved).is_err());
        assert!(!path.exists());
        saved.version = 1;
        saved.report.operations[0].status = "running".into();
        assert!(save(&path, &saved).is_err());
        assert!(!path.exists());
        saved.report.operations[0].status = "succeeded".into();
        saved.report.operations[0].error = "x".repeat(LIMIT as usize);
        assert!(save(&path, &saved).is_err());
        assert!(!path.exists());
        fs::write(&path, b"private invalid state").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            load(&path).err().unwrap().to_string(),
            "Invalid backend update state"
        );
        assert_eq!(fs::read(&path).unwrap(), b"private invalid state");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(load(&link).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"private invalid state");
    }
}
