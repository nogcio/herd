//! Profiles and runtime observations are distinct; neither proves idle inference.
use crate::icons::IconSpec;
use serde::Serialize;
use std::collections::BTreeMap;

/// A native preset or a cache entry reported by the router.
#[derive(Clone, Debug, Serialize)]
pub struct Model {
    /// Native preset section ID or router cache ID.
    pub id: String,
    /// Optional presentation label, otherwise the ID.
    pub name: String,
    /// Compact presentation label.
    pub short_name: String,
    /// Native llama.cpp arguments, not a second runtime registry.
    pub settings: BTreeMap<String, String>,
    /// Optional decorative artwork; never lifecycle authority.
    #[serde(skip)]
    pub icon: IconSpec,
}

impl Model {
    /// Small panel badge; does not identify loaded weights.
    pub fn badge(&self) -> String {
        if self.short_name.is_empty() {
            self.id
                .rsplit('/')
                .next()
                .unwrap_or(&self.id)
                .chars()
                .take(2)
                .collect::<String>()
                .to_uppercase()
        } else {
            self.short_name.clone()
        }
    }
}

/// Observed router state, never a claim that interrupting a model is safe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", content = "detail", rename_all = "snake_case")]
pub enum State {
    /// Not loaded.
    Stopped,
    /// Loading into memory.
    Starting,
    /// Router reports the child loaded.
    Ready,
    /// Loaded child has entered runtime sleep.
    Sleeping,
    /// Download byte counts from the router, not estimated network usage.
    Downloading {
        /// Logical bytes downloaded.
        done: u64,
        /// Total logical bytes reported by the router.
        total: u64,
    },
    /// A download is paused or incomplete.
    Incomplete,
    /// Unload requested.
    Stopping,
    /// Child exited unsuccessfully.
    Failed,
    /// Router or child readiness is unavailable.
    Unavailable,
    /// Unrecognized state; new starts must fail closed.
    Unknown(String),
}

impl State {
    /// A load can only be admitted from a non-running, non-downloading state.
    pub fn can_start(&self) -> bool {
        matches!(self, Self::Stopped | Self::Failed)
    }
    /// Compact English presentation, not a machine-readable protocol.
    pub fn label(&self) -> &str {
        match self {
            Self::Stopped => "Stopped",
            Self::Starting => "Loading…",
            Self::Ready => "Ready",
            Self::Sleeping => "Sleeping",
            Self::Downloading { .. } => "Downloading…",
            Self::Incomplete => "Download incomplete",
            Self::Stopping => "Stopping…",
            Self::Failed => "Load failed",
            Self::Unavailable => "Router unavailable",
            Self::Unknown(_) => "Unknown state",
        }
    }
}

/// Profile plus its latest observation. Configuration is not running state.
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    /// Native profile or router cache entry.
    pub model: Model,
    /// Most recent router observation.
    pub state: State,
    /// Safe operational context, never upstream raw response or process arguments.
    pub detail: Option<String>,
}
