# ADR 0003: User-session D-Bus management backend

Status: implemented in source; real user-unit activation, router scope lifetime
and native presentation require interactive acceptance.

## Decision

Use one `herd-service` process, registered as a systemd user
service named `herd.service`, with `Type=dbus` and the
well-known session-bus name `io.github.nogcio.Herd.Backend`.
The interface is `io.github.nogcio.Herd.Backend1`, at
`/io/github/nogcio/Herd/Backend`. It exposes API version 1.

D-Bus activation starts the installed user unit on first access. Explicit login
autostart is optional and never enabled by installation. The separate visible
application keeps its own single-instance activation identity; the hidden applet
entry is panel-hosted. Clients never launch the backend themselves or fall back
to direct configuration/runtime management.

This is a local management API, not an inference proxy. No HTTP endpoint,
custom socket framing, system service, root helper, or remote access is added.
The normal user-session bus is a same-user boundary, not isolation from other
applications running as that user.

## API and operation ownership

Typed methods expose configuration, model observations, public metadata and
explicit operations. Native D-Bus structs carry strings, arrays, dictionaries,
integers and booleans. JSON is the non-interactive CLI's output format only.

`EditPreset` returns a profile's explicit options, shared native defaults and
the full-file revision. `SavePreset` replaces only that profile's explicit
options in one validated, revision-guarded operation. The UI never reconstructs
profiles from flattened runtime observations or saves a form one option at a
time. Single-option CLI edits delegate to the same atomic preset writer.

Mutations admit one service-owned task at a time and return its ID immediately.
The caller disconnecting does not cancel that task. Confirmation is checked at
the service boundary for unload, router shutdown and acquisition; existing
controller admission still verifies current presets, artifacts and competitors.
Search and file review do not download weights. Backend activation never starts
a router, model, or transfer.

An acquisition operation completes when its explicit native submission finishes,
not when all bytes arrive. llama.cpp owns subsequent progress, cache and partial
files. Pause remains available after submission. Verification is a separate
service-owned operation; a caller waiting for completion does not own transfer
lifetime or authorize implicit resume.

The backend retains at most 64 operations in memory, with instance-qualified IDs
and running/succeeded/failed outcomes. There is no durable command queue.
Restart does not replay anything. A coordinated executable update transfers
terminal results to its replacement; an ordinary restart or history eviction
still makes old IDs produce an explicit unknown-outcome error. A failed call or
lost acknowledgement is not proof that no mutation occurred. Clients must
observe state before an explicit retry. See
[coordinated backend updates](0007-backend-update.md).

## Observation and lifetime

The service watches authoritative configuration directories, coalesces changes
for 200 ms, and periodically reconciles router observations. Invalid data keeps
previous model observations with an explicit error, blocking new starts.
`Changed` and `OperationChanged` signals notify clients; a bounded full snapshot
is authoritative. Clients subscribe and reread after connecting and periodically
reconcile, rather than treating signals as a reliable event journal.

Hosting preferences are pinned to the first valid configuration in a backend
instance. A watcher cannot silently switch endpoints/state directories and hide
the previous host. Changed or unreadable preferences retain stale observations
and block new work; explicit unload/shutdown can still address the previously
known host. Stop the old host explicitly before applying new hosting settings
and restarting the backend. Native preset edits remain independently
live-observed.

Process Compose continues to host the router using the reviewed native manifest.
Explicit startup runs through `systemd-run --user --scope` with a stable
state-directory-derived unit name. This puts the router outside the backend
service's cgroup; ordinary detached spawning alone would not establish this
property. Backend stop/restart never invokes router shutdown or model unload.
User logout and session-manager policy remain separate lifetime boundaries.

The service's private-bus subprocess test covers singleton admission, typed
signals, caller disconnection, conflict failures with unchanged competitors and
restart without command replay. It does not certify real systemd activation or
cgroup survival; those require explicitly authorized native acceptance.
