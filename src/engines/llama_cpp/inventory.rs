//! llama.cpp inventory retains its native parser and router response admission.
use super::{
    config::{Presets, Settings},
    runtime::Runtime,
};
use crate::{
    engines::inventory::{self, Kind},
    runtime::RuntimeRecord,
};
use std::{ffi::OsStr, path::Path};

pub(crate) struct Inspection {
    record: RuntimeRecord,
    settings: Option<Settings>,
}

pub(crate) fn inspect(path: &Path, search: Option<&OsStr>) -> Inspection {
    let mut record = inventory::record(Kind::LlamaCpp, path);
    record.executable_found = inventory::on_path("llama-server", search);
    let settings = match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            record.state = "stopped".into();
            None
        }
        _ => match Settings::read_path(path) {
            Ok(settings) => {
                record.executable_found = inventory::executable(&settings.server);
                match Presets::read(&settings.presets) {
                    Ok(_) => {
                        record.configuration = "valid".into();
                        record.endpoint.clone_from(&settings.endpoint);
                        Some(settings)
                    }
                    Err(_) => {
                        record.configuration = "invalid".into();
                        record.error = "llama.cpp native presets are unreadable or invalid".into();
                        None
                    }
                }
            }
            Err(_) => {
                record.configuration = "invalid".into();
                record.error = "llama.cpp hosting preferences are unreadable or invalid".into();
                None
            }
        },
    };
    Inspection { record, settings }
}

pub(crate) async fn observe(mut inspection: Inspection) -> RuntimeRecord {
    if let Some(settings) = inspection.settings {
        match async { Runtime::new(&settings)?.list().await }.await {
            Ok(entries) => {
                inspection.record.observation = "reachable".into();
                inspection.record.state = if entries
                    .iter()
                    .all(|e| e.state.can_start() || e.state == crate::model::State::Incomplete)
                {
                    "stopped"
                } else {
                    "busy"
                }
                .into();
            }
            Err(_) => {
                inspection.record.observation = "unknown".into();
                inspection.record.error =
                    "llama.cpp router observation failed; no stopped state is inferred".into();
                let endpoint = url::Url::parse(&settings.endpoint).ok();
                if settings
                    .state_dir
                    .join("router.sock")
                    .symlink_metadata()
                    .is_err()
                    && endpoint.and_then(|u| u.port()).is_some_and(|p| {
                        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, p)).is_ok()
                    })
                {
                    inspection.record.state = "stopped".into();
                }
            }
        }
    }
    inspection.record
}
