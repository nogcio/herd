//! Native D-Bus structures; no JSON envelope or inference traffic on this API.
use crate::{
    config::Settings,
    icons::IconSpec,
    model::{Model, Snapshot, State},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zbus::zvariant::Type;

/// A model/profile observation with bounded native options and decorative metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ModelRecord {
    /// Native profile or router entry identity.
    pub id: String,
    /// Presentation name.
    pub name: String,
    /// Compact badge text.
    pub short_name: String,
    /// Effective native INI options, not a separate registry.
    pub settings: BTreeMap<String, String>,
    /// Optional local decorative artwork, empty when absent.
    pub icon_local: String,
    /// Optional remote artwork with upstream provenance, empty when absent.
    pub icon_url: String,
    /// Stable state identifier, never presentation text.
    pub state: String,
    /// Downloaded logical bytes.
    pub done: u64,
    /// Total logical bytes.
    pub total: u64,
    /// Safe diagnostic or pending-configuration context.
    pub detail: String,
}
impl From<Snapshot> for ModelRecord {
    fn from(s: Snapshot) -> Self {
        let (state, done, total) = match s.state {
            State::Stopped => ("stopped", 0, 0),
            State::Starting => ("starting", 0, 0),
            State::Ready => ("ready", 0, 0),
            State::Sleeping => ("sleeping", 0, 0),
            State::Downloading { done, total } => ("downloading", done, total),
            State::Incomplete => ("incomplete", 0, 0),
            State::Stopping => ("stopping", 0, 0),
            State::Failed => ("failed", 0, 0),
            State::Unavailable => ("unavailable", 0, 0),
            State::Unknown(_) => ("unknown", 0, 0),
        };
        Self {
            id: s.model.id,
            name: s.model.name,
            short_name: s.model.short_name,
            settings: s.model.settings,
            icon_local: s.model.icon.local.unwrap_or_default(),
            icon_url: s.model.icon.url.unwrap_or_default(),
            state: state.into(),
            done,
            total,
            detail: s.detail.unwrap_or_default(),
        }
    }
}
impl From<ModelRecord> for Snapshot {
    fn from(r: ModelRecord) -> Self {
        let optional = |s: String| (!s.is_empty()).then_some(s);
        let state = match r.state.as_str() {
            "stopped" => State::Stopped,
            "starting" => State::Starting,
            "ready" => State::Ready,
            "sleeping" => State::Sleeping,
            "downloading" => State::Downloading {
                done: r.done,
                total: r.total,
            },
            "incomplete" => State::Incomplete,
            "stopping" => State::Stopping,
            "failed" => State::Failed,
            "unavailable" => State::Unavailable,
            _ => State::Unknown(r.state),
        };
        Self {
            model: Model {
                id: r.id,
                name: r.name,
                short_name: r.short_name,
                settings: r.settings,
                icon: IconSpec {
                    local: optional(r.icon_local),
                    url: optional(r.icon_url),
                },
            },
            state,
            detail: optional(r.detail),
        }
    }
}

/// Common observations; invalid configuration retains explicitly stale models.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Report {
    /// Monotonic revision within this backend instance.
    pub revision: u64,
    /// Last successfully observed models.
    pub models: Vec<ModelRecord>,
    /// Current observation/configuration failure, empty on success.
    pub error: String,
    /// Initial hosting preferences are absent, with no previously pinned host.
    pub setup_required: bool,
    /// At most one running operation, plus bounded completed history.
    pub operations: Vec<Operation>,
}

/// Service-owned mutation, retained independently of connected clients.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Operation {
    /// Instance-qualified identity; invalid after a service restart.
    pub id: String,
    /// Concrete operation name.
    pub kind: String,
    /// Target identity, empty for host/setup operations.
    pub model: String,
    /// Running, succeeded, or failed; failure can have an uncertain runtime outcome.
    pub status: String,
    /// Safe diagnostic, empty unless failed.
    pub error: String,
}

/// One authoritative native configuration snapshot for conflict-checked editing.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct Configuration {
    /// Absolute native INI location.
    pub path: String,
    /// Exact native-file SHA-256.
    pub revision: String,
    /// Native profiles, without claiming observed readiness.
    pub models: Vec<ModelRecord>,
}

/// Reviewed removal scope; profiles never grant ownership of external GGUF files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RemovalPlan {
    /// Exact target identity.
    pub model: String,
    /// Native configuration revision for concurrent-change detection.
    pub revision: String,
    /// True only for native cache deletion; false unregisters a profile only.
    pub delete_files: bool,
}

/// One editable native profile, preserving explicit versus inherited options.
#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
pub struct PresetDraft {
    /// Native profile identity; renaming is not part of an option save.
    pub id: String,
    /// Exact full-file revision used for atomic admission.
    pub revision: String,
    /// Explicit options in this profile only.
    pub options: BTreeMap<String, String>,
    /// Shared native defaults, read-only in a per-model editor.
    pub defaults: BTreeMap<String, String>,
}

/// Hosting preferences transported as native D-Bus fields, not executed commands.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct Preferences {
    /// Installed llama-server executable.
    pub server: String,
    /// Runtime shared-library directories.
    pub library_dirs: Vec<String>,
    /// Installed Process Compose executable.
    pub process_compose: String,
    /// Native preset path.
    pub presets: String,
    /// Explicit loopback origin.
    pub endpoint: String,
    /// Private operational state directory.
    pub state_dir: String,
    /// Working directory for the router.
    pub working_dir: String,
}
impl From<Preferences> for Settings {
    fn from(p: Preferences) -> Self {
        Self {
            server: p.server.into(),
            library_dirs: p.library_dirs.into_iter().map(Into::into).collect(),
            process_compose: p.process_compose.into(),
            presets: p.presets.into(),
            endpoint: p.endpoint,
            state_dir: p.state_dir.into(),
            working_dir: p.working_dir.into(),
        }
    }
}
impl From<Settings> for Preferences {
    fn from(s: Settings) -> Self {
        Self {
            server: s.server.to_string_lossy().into_owned(),
            library_dirs: s
                .library_dirs
                .into_iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            process_compose: s.process_compose.to_string_lossy().into_owned(),
            presets: s.presets.to_string_lossy().into_owned(),
            endpoint: s.endpoint,
            state_dir: s.state_dir.to_string_lossy().into_owned(),
            working_dir: s.working_dir.to_string_lossy().into_owned(),
        }
    }
}

/// Public Hub metadata with an empty license string for absent declarations.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct Catalogue {
    /// Canonical repository identity.
    pub repo: String,
    /// Inspected commit.
    pub revision: String,
    /// Declared model card license, if any.
    pub license: String,
    /// Admitted single-file choices.
    pub plans: Vec<crate::downloads::Plan>,
}
impl From<crate::downloads::Catalogue> for Catalogue {
    fn from(c: crate::downloads::Catalogue) -> Self {
        Self {
            repo: c.repo,
            revision: c.revision,
            license: c.license.unwrap_or_default(),
            plans: c.plans,
        }
    }
}
impl From<Catalogue> for crate::downloads::Catalogue {
    fn from(c: Catalogue) -> Self {
        Self {
            repo: c.repo,
            revision: c.revision,
            license: (!c.license.is_empty()).then_some(c.license),
            plans: c.plans,
        }
    }
}
