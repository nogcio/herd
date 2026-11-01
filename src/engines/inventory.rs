//! Passive discovery only; native configuration never grants process ownership.
use super::{colibri, llama_cpp};
use crate::runtime::RuntimeRecord;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    LlamaCpp,
    Colibri,
}

impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::LlamaCpp => "llama_cpp",
            Self::Colibri => "colibri",
        }
    }
}

pub(crate) fn record(kind: Kind, path: &Path) -> RuntimeRecord {
    let engine = kind.name();
    let digest = hex::encode(Sha256::digest(path.as_os_str().as_encoded_bytes()));
    RuntimeRecord {
        id: format!("{engine}:{digest}"),
        engine: engine.into(),
        executable_found: false,
        configuration: "absent".into(),
        observation: "not_observed".into(),
        endpoint: String::new(),
        profile: String::new(),
        name: String::new(),
        revision: String::new(),
        state: "unknown".into(),
        managed: false,
        can_start: false,
        can_load_acquired: false,
        can_stop: false,
        pending: false,
        transfer: String::new(),
        transfer_model: String::new(),
        transfer_fingerprint: String::new(),
        done: 0,
        total: 0,
        error: String::new(),
    }
}

pub(super) fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_absolute()
        && path
            .metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub(super) fn on_path(name: &str, search: Option<&OsStr>) -> bool {
    search
        .is_some_and(|search| std::env::split_paths(search).any(|dir| executable(&dir.join(name))))
}

pub(crate) fn find(name: &str) -> anyhow::Result<PathBuf> {
    std::env::var_os("PATH")
        .and_then(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join(name))
                .find(|p| executable(p))
        })
        .ok_or_else(|| {
            anyhow::anyhow!("Required installed executable is missing from the backend PATH")
        })
}

pub(crate) fn host_directory() -> anyhow::Result<PathBuf> {
    let path = llama_cpp::config::Settings::path()?;
    Ok(path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cannot locate host state directory"))?
        .join("colibri-host"))
}

/// Locations are captured by the service, never by a frontend or subprocess.
pub(crate) struct Locations {
    pub llama_cpp: anyhow::Result<PathBuf>,
    pub colibri: anyhow::Result<PathBuf>,
    pub search_path: Option<std::ffi::OsString>,
    pub host_directory: anyhow::Result<PathBuf>,
}

impl Locations {
    pub fn capture() -> Self {
        Self {
            llama_cpp: llama_cpp::config::Settings::path(),
            colibri: colibri_location(
                std::env::var_os("COLI_SETUP_HOME"),
                std::env::var_os("XDG_DATA_HOME"),
                dirs::home_dir(),
            ),
            search_path: std::env::var_os("PATH"),
            host_directory: host_directory(),
        }
    }
}

fn colibri_location(
    setup_home: Option<std::ffi::OsString>,
    data_home: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> anyhow::Result<PathBuf> {
    use anyhow::{Context, ensure};
    let home = match setup_home.filter(|v| !v.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => match data_home.filter(|v| !v.is_empty()) {
            Some(data) => PathBuf::from(data).join("colibri"),
            None => home
                .context("Cannot locate Colibri data directory")?
                .join(".local/share/colibri"),
        },
    };
    ensure!(
        home.is_absolute(),
        "Colibri setup directory must be absolute; tilde paths must be expanded explicitly"
    );
    Ok(home.join("setup.json"))
}

pub(crate) async fn observe(locations: Locations) -> anyhow::Result<Vec<RuntimeRecord>> {
    // Regular-file reads may block on a filesystem. Keep them off the async bus executor.
    let (llama, colibri) = tokio::task::spawn_blocking(move || {
        (
            locations
                .llama_cpp
                .map(|path| llama_cpp::inventory::inspect(&path, locations.search_path.as_deref())),
            locations.colibri.and_then(|path| {
                locations.host_directory.map(|directory| {
                    colibri::inspect(&path, locations.search_path.as_deref(), &directory)
                })
            }),
        )
    })
    .await
    .map_err(|_| anyhow::anyhow!("Runtime inventory file task failed"))?;
    let (llama, colibri) = tokio::join!(
        async {
            match llama {
                Ok(inspection) => llama_cpp::inventory::observe(inspection).await,
                Err(_) => unlocated(Kind::LlamaCpp),
            }
        },
        async {
            match colibri {
                Ok(inspection) => colibri::observe(inspection).await,
                Err(_) => unlocated(Kind::Colibri),
            }
        }
    );
    let mut colibri = colibri;
    colibri.can_load_acquired = colibri.state == "stopped"
        && colibri.configuration != "invalid"
        && colibri.transfer == "complete"
        && llama.configuration != "invalid"
        && llama.state == "stopped";
    if colibri.can_start && (llama.configuration == "invalid" || llama.state != "stopped") {
        colibri.can_start = false;
        colibri.error =
            "llama.cpp has an active or uncertain competitor; stop it explicitly first".into();
    }
    Ok(vec![llama, colibri])
}

fn unlocated(kind: Kind) -> RuntimeRecord {
    let mut record = record(kind, Path::new(""));
    record.id = format!("{}:unlocated", kind.name());
    record.configuration = "invalid".into();
    record.error = "Native configuration location is unavailable or is not absolute".into();
    record
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_location_precedence_never_falls_back_from_an_invalid_explicit_source() {
        let home = Some(PathBuf::from("/example/home"));
        assert_eq!(
            colibri_location(
                Some("/example/setup".into()),
                Some("/example/data".into()),
                home.clone()
            )
            .unwrap(),
            Path::new("/example/setup/setup.json")
        );
        assert_eq!(
            colibri_location(None, Some("/example/data".into()), home.clone()).unwrap(),
            Path::new("/example/data/colibri/setup.json")
        );
        assert_eq!(
            colibri_location(Some("".into()), Some("".into()), home.clone()).unwrap(),
            Path::new("/example/home/.local/share/colibri/setup.json")
        );
        for invalid in ["relative", "~/colibri"] {
            assert!(
                colibri_location(
                    Some(invalid.into()),
                    Some("/example/data".into()),
                    home.clone()
                )
                .is_err()
            );
            assert!(colibri_location(None, Some(invalid.into()), home.clone()).is_err());
        }
        assert!(colibri_location(None, None, None).is_err());
    }

    #[test]
    fn identities_are_scoped_by_engine_and_location_not_configuration_contents() {
        let a = Path::new("/example/first.json");
        let b = Path::new("/example/second.json");
        assert_eq!(record(Kind::Colibri, a).id, record(Kind::Colibri, a).id);
        assert_ne!(record(Kind::Colibri, a).id, record(Kind::LlamaCpp, a).id);
        assert_ne!(record(Kind::Colibri, a).id, record(Kind::Colibri, b).id);
    }

    #[tokio::test]
    async fn an_invalid_location_does_not_hide_another_engine_or_change_its_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("server.json");
        std::fs::write(&file, b"invalid hosting preferences").unwrap();
        let records = observe(Locations {
            llama_cpp: Ok(file.clone()),
            colibri: Err(anyhow::anyhow!("private location error")),
            search_path: None,
            host_directory: Ok(dir.path().join("host")),
        })
        .await
        .unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].engine, "llama_cpp");
        assert_eq!(records[0].configuration, "invalid");
        assert_eq!(records[1].engine, "colibri");
        assert_eq!(records[1].id, "colibri:unlocated");
        assert_eq!(records[1].configuration, "invalid");
        assert!(!records[1].error.contains("private location error"));
        assert_eq!(std::fs::read(file).unwrap(), b"invalid hosting preferences");
    }
}
