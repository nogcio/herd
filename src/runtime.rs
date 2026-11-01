//! Engine-qualified runtime observations, separate from model readiness.
use serde::{Deserialize, Serialize};
use zbus::zvariant::Type;

/// One native configuration and its separately observed instance and capabilities.
/// Reachability proves neither process ownership nor model compatibility.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RuntimeRecord {
    /// Engine-qualified identity derived from the native configuration location.
    pub id: String,
    /// Concrete integration identifier, not an inference protocol name.
    pub engine: String,
    /// A launcher was found; does not establish a complete or working installation.
    pub executable_found: bool,
    /// Native configuration state: absent, valid, or invalid.
    pub configuration: String,
    /// Endpoint observation: not_observed, reachable, or unknown.
    pub observation: String,
    /// Validated loopback origin, empty when configuration is unavailable.
    pub endpoint: String,
    /// Native configured profile identity, empty if unavailable.
    pub profile: String,
    /// Presentation label, not an artifact or process identity.
    pub name: String,
    /// Exact native configuration revision used to admit a reviewed start.
    pub revision: String,
    /// Instance observation: stopped, busy, starting, ready, external, failed or unknown.
    pub state: String,
    /// True only after matching the recorded private host's process identity.
    pub managed: bool,
    /// Current action availability; execution always rechecks fresh state.
    pub can_start: bool,
    /// Current admission for explicitly selecting and loading an acquired artifact.
    pub can_load_acquired: bool,
    /// Current ownership-based stop availability, independent of edited configuration.
    pub can_stop: bool,
    /// Native configuration differs from the running instance; saving never restarts it.
    pub pending: bool,
    /// Native acquisition state, independent of inference lifecycle.
    pub transfer: String,
    /// Native catalogue identity currently being acquired.
    pub transfer_model: String,
    /// Reviewed acquisition identity, independent of the selected profile.
    pub transfer_fingerprint: String,
    /// Observed logical bytes, never an inferred bandwidth quota.
    pub done: u64,
    /// Total advertised acquisition bytes.
    pub total: u64,
    /// Sanitized diagnostic; never native environment, arguments or response bodies.
    pub error: String,
}

/// Runtime-qualified profile selection; a native model name alone is not unique.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Target {
    /// Configuration-location identity returned by the service.
    pub runtime: String,
    /// Native model/profile identity within that runtime.
    pub model: String,
    /// Reviewed configuration revision, required for Colibri's selected setup.
    pub revision: String,
}
