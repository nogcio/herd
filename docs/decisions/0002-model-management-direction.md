# ADR 0002: Native model control and independent router hosting

Status: accepted for local artifact loading, native configuration, independent
hosting, GUI/CLI controls, live observation and explicit single-file
acquisition.

## Decision

- Use llama.cpp's native router and INI presets instead of a service frontend or
  an application-owned model database.
- Use Process Compose's daemonization, Unix IPC and group termination instead of
  building a supervisor or retaining a PID-file launcher.
- Keep the panel compact. Use a standalone libcosmic management window for
  settings, not a fork of COSMIC Settings. Its external plugin mechanism is not
  implemented in the [reviewed upstream source][settings].
- GUI and CLI invoke the
  [shared user-session backend](0003-user-session-backend.md). It calls the
  concrete runtime operations. There are no published API/data compatibility
  obligations requiring a legacy systemd branch or migration layer.
- Reuse existing GGUF files. A configuration edit, registration or observation
  never authorizes a download, eviction or restart.

## Runtime contract

The supported server is b11856, commit `3642238cc`; the
[pinned native documentation][server] and source define the API. Process Compose
v1.122.0 supplies the hosting contract. Native presets own per-model parameters;
hosting preferences only identify binaries, paths and the loopback endpoint.

`GET /models` is passive. The application only calls `?reload=1` within explicit
start admission after observing all models stopped, since source changes can
otherwise unload a running instance. File watchers never call it. Native
autoload is disabled and `models-max=0` disables LRU-based eviction. Application
admission refuses concurrent models rather than automatically stopping one.

Native directory watching handles manual edits and atomic saves. Invalid
configuration is visible and blocks starts. Edits remain pending for running
models and are applied only during a later explicit, safely admitted start.

Normal inference children receive offline mode. Explicit acquisition uses native
download workers without inheriting inference presets. The interface reviews
unambiguous single-file GGUF metadata and records the expected digest, then
requires explicit authorization. Native transfer follows current main:
inspection does not prove an immutable fetch or hard transfer byte bound. Size
admission is checked before submission and the reviewed digest must match before
loading. Do not duplicate native downloads or silently select sidecars/split
files. See the [acquisition contract](../downloads.md) for current limits.

## Consequences and limits

Closing UI does not own router/model lifetime. Process Compose starts within a
separate user scope, not the backend service cgroup. Host shutdown is an
explicitly confirmed operation affecting all its models. Cross-process
application locking does not control arbitrary external runtime clients. Runtime
state is not proof that a model is idle, compatible, or serving correct
inference.

The application does not reproduce systemd cgroup memory policies. Existing
resource constraints require explicit owner review when changing hosting; never
silently discard them as if native inference presets were equivalent.

Real user service cleanup is separate from source replacement. Back up and
inventory authorized live files, verify the new path, preserve weights, and
delete only the owner's explicitly scoped obsolete files. Do not build a
permanent compatibility framework solely for an authorized live transfer.

[server]: https://github.com/ggml-org/llama.cpp/blob/3642238cc/tools/server/README.md#using-multiple-models
[settings]: https://github.com/pop-os/cosmic-settings/blob/f1d00fc06e18203e880771617657067c10c43321/cosmic-settings/src/app.rs#L645-L646
