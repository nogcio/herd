# Architecture contract

## Scope and owners

One runtime Rust package plus a repository-only source-check xtask. Native
COSMIC views and a headless CLI share concrete runtime operations; there is no
generic backend, application model database, compatibility adapter, or inference
proxy.

Arch packaging owns installation and updates; there is no custom installer or
service-restart package hook. See
[distribution ownership](decisions/0005-arch-packaging.md).

```text
panel / native management application / CLI
    -> typed session D-Bus -> systemd user backend service
        -> Controller -> Runtime -> loopback llama.cpp router
        -> native Presets / file watcher / bounded common snapshot
        -> service-owned operations / bounded operation history
        -> Process Compose host commands in an independent user scope
    -> decorative artwork presentation inputs
```

| Owner | Responsibility |
| --- | --- |
| `config` | Bounded native presets, hosting preferences, atomic edits, watcher |
| `backend` | Typed D-Bus API/client, shared observations, service-owned operations |
| `runtime` | Bounded router JSON transport; no process inspection or inference |
| `downloads` | Public Hub inspection, acquisition receipts and digest checks |
| `controller` | Fresh admission, shared operation lock, explicit load/unload |
| `hosting` | Reviewed Process Compose daemon, private IPC/logs, router startup |
| `model` | Profile and observed state values |
| `icons` | Bounded decorative artwork loading and cache |
| `regular_file` | Nonblocking open and descriptor-based regular-file validation |
| `ui` | Native windows, popup, async state and edits, not runtime policy |
| `bin` | Headless command admission and output |

The application, panel and CLI do not instantiate controllers, mutate native
configuration, or call runtime/Hub management transport. See the
[backend contract](decisions/0003-user-session-backend.md). Decorative artwork
is a presentation input, not management authority. Each client call checks API
version 1 against the unique bus owner receiving the requested method;
owner loss fails the call without replaying it.

Frontend startup additionally requests a coordinated update if the installed
management executable replaced the running one. The backend closes mutation
admission, completes accepted work, and writes a private one-shot handoff of
terminal results and pinned host observations. The user service restarts under
systemd, consumes that handoff only after acquiring the bus name, and reobserves
the unchanged router. CLI observation remains passive. See
[backend updates](decisions/0007-backend-update.md).

## Configuration authority

Native INI presets are the only model settings source. `server.json` specifies
executable/library paths, working directory, native preset path, endpoint, and
private host state directory. It contains no model list. See
[presets](presets.md). No runtime configuration is inferred from launch scripts
or process arguments.

The backend directory watcher handles file replacement as well as in-place
edits. Events are bounded and coalesced for 200 ms; the backend rereads
validated configuration. An ordinary status refresh never calls
`/models?reload=1` or starts anything. Periodic observations also recover from
dropped/coalesced events.

An edit uses a SHA-256 token, preserves unrelated file lines, validates the new
file, rechecks the original revision, retains an exact private preset backup,
and atomically replaces it. External
non-cooperating editors can still race the final check/rename; this is
optimistic conflict detection, not a filesystem compare-and-swap guarantee.

Native literal validation rejects `#` and `;` comment markers in values,
ambiguous section-name whitespace and indented headers/options. Reads validate
an opened regular-file descriptor using nonblocking open flags, rejecting
special files without FIFO waits. This is not a hard read deadline for arbitrary
regular filesystems. Preset mutations reject symlinked preset paths; artifact
symlinks resolving to regular files remain supported.

## Admission and lifetime

- Construction and observation do not start a server, model, or download.
  Opening the management window requests empty-router startup once after valid
  setup; this is a service-owned operation, not a side effect of status
  observation.
- One service-owned mutation is admitted at a time, independently of connected
  clients. The existing private file lock additionally protects runtime
  admission.
- Starts validate fresh presets and every router entry, including unmanaged
  cache entries. An active, downloading, sleeping, or unknown competitor blocks
  a start. There is no automatic eviction.
- Explicit starts may refresh native presets only after observing all children
  stopped. Configuration is rechecked before loading. Watchers never send that
  potentially destructive reload request.
- Configuration hashes in private host state mark pending changes; they are
  operational state, not a duplicate model registry. Editing does not change the
  parameters of an existing instance.
- Unload confirmation belongs to the caller. Invalid edited configuration does
  not prevent explicitly stopping an already observed runtime entry.
- A timeout or dropped UI future does not issue a compensating unload. Request
  failure is not proof that the router rejected the action; observe current
  state before retrying. Mutating requests are never automatically retried.
- External clients that bypass the application lock remain independent. The
  lock is not a machine-wide GPU reservation or an idle-session detector.
- Removal is service-owned and explicitly confirmed. Profile removal atomically
  edits native INI only, retaining original weights and every other profile.
  Native cache deletion requires fresh idle observations; it never sends a stop
  or pause as part of deletion. The runtime owns cache/partial-file cleanup.

Process Compose owns the router process group and daemonization. llama.cpp owns
model children. An independent systemd user scope owns the host cgroup; the
backend service does not. Windows and panel state own neither. The router uses
loopback, `--no-models-autoload`, `--models-max 0` (disables native LRU
eviction), and `--offline`. Profiles cannot opt into startup loading. Shared
native parameters are supplied by presets, not router CLI overrides.

The host uses argv entrypoints rather than shell command strings. Its constant
launcher sets `umask 077`; user paths are passed as quoted arguments. Dotenv,
command rendering, implicit restarts, and file-triggered restarts are disabled.
Process Compose's template-disable setting applies only to commands, not working
directories, log locations or environment entries. Rendered path/environment
values containing `{{` or `}}` are unsupported and rejected before launch. Host
shutdown uses a private Unix socket, not process-name searches or PID files.
Host startup invokes `systemd-run --user --scope`; backend activation and
restart never start a router or replay a previous command. The window requests
startup once per launch, including after first-run setup. Concurrent requests
share an in-flight startup. A responsive router with matching host identity is
retained without reload, unload or restart. Failures require a manual retry;
timers and watcher events do not supervise or resurrect the host. The panel and
CLI remain passive until an explicit action. See
[startup and runtime dependencies](decisions/0006-runtime-startup.md).

## Automatic native configuration

b11856 has no supported flag disabling automatic `config.ini` loading, including
for version probes and model/download children. Hosting therefore checks
`/etc/llama.cpp/config.ini` and the exact host-user path:
`$XDG_CONFIG_HOME/llama.cpp/config.ini`, or
`$HOME/.config/llama.cpp/config.ini` when XDG configuration is unset. The host
requires absolute HOME and configuration paths. If either automatic path exists
or its absence cannot be established, executable version probes and router
launch fail closed. Herd does not read the contents, delete these files or move
HOME/cache to evade them.

After readiness, private `cache-root.json` records the native cache root, both
frozen automatic-config paths and a digest of every effective hosting setting,
including preset path, endpoint, executable, libraries and working directory.
Before reload, load or download-child dispatch, the controller rechecks those
recorded host paths, not paths derived from a reconnecting backend's
environment. Missing or mismatched identity blocks new model starts and cache
operations while preserving existing inference. An explicitly authorized router
replacement is required to establish identity; no stop or restart is automatic.

These are admission checks, not atomic namespace isolation. Independent writers
can create automatic configuration after a check and before native execution;
that race is unsupported. No guarantee against external filesystem writers is
claimed.

## Explicit acquisition

Read-only public Hub metadata uses bounded HTTPS requests. A selected
single-file GGUF plan is refreshed before admission; unsupported or ambiguous
file sets fail closed. The controller submits `POST /models` with
`sidecars: []`, never a load. Pause addresses only a currently downloading
entry. Inference competitors and other transfers block a new acquisition; no
automatic unload is sent.

Acquisition receipts under private host state are operational integrity records,
not profiles or a duplicate runtime cache index. An artifact acquired by the
application must match its reviewed size/header/SHA-256 before it can load.
Verification runs off the UI executor. Native cache/partial files and transfer
workers remain owned by llama.cpp even after all UI processes exit.

In b11856, `GET /models` reports download state but omits byte progress. The
backend subscribes to the read-only `/models/sse` native `download_progress`
events using the maintained `eventsource-stream` parser. It merges sanitized
counts into downloading observations only, bounds each connection to 2 MiB and
60 seconds, and reconnects without replaying a mutation. Progress loss produces
an indeterminate indicator, not a claim that zero bytes were transferred. URLs
never enter the typed frontend snapshot. A periodic list remains authoritative
for lifecycle state. Events for inactive IDs are ignored, so stale progress
cannot resurrect a paused/deleted model.

The pinned `/models` response has no artifact path. Read-only native HF cache
inspection resolves the requested repository's `refs/main` and current
`snapshots/<revision>` only, with strict unambiguous single-file selection and
acquisition-receipt digest checks. After explicit host startup reaches
readiness, `cache-root.json` records the selected native cache root and
[host identity](#automatic-native-configuration) in private state. It is
operational identity, not a model registry. Cache-dependent controller
operations and new model starts fail closed with missing or mismatched identity
until an explicitly authorized router replacement establishes it; they never
automatically restart the router. See
[cache resolution](downloads.md#native-cache-resolution).

The native API follows current main, not the inspected immutable revision; size
admission cannot enforce a bandwidth quota. Never treat HTTP acceptance as
completion. Normal inference entries receive offline mode from the base preset;
explicit download workers have a separate native acquisition mode without those
inference presets. See the [acquisition contract](downloads.md).

## Verified dependency boundary

The reviewed reference llama.cpp build is b11856, commit `3642238cc`; Process
Compose v1.122.0 defines the reviewed daemon/IPC/termination mechanisms. Neither
version is an admission requirement: executable probes check successful exit,
not a version string. Other builds can have incompatible flags or API semantics;
accepting their version is not verification of offline/autoload/eviction safety
on those builds. Management transport still validates responses and never
falls back to a different lifecycle policy. Cargo
dependencies are pinned by Cargo.lock. Source and license/advisory checks remain
separate from compilation. See [third-party obligations](third-party.md).

Router readiness, loaded state, inference quality, and absence of active client
requests are different guarantees. The application currently establishes only
management readiness and router-reported model state. Native rendering still
requires actual panel input and frames; see [testing](testing.md).
