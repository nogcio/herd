# ADR 0007: Coordinated management-backend updates

Status: implemented in source; native upgrade acceptance is separate.

## Ownership and trigger

Pacman continues to own installation. Arch's existing systemd hook rereads user
unit metadata; Herd adds no privileged package hook, updater or process
supervisor. The old management service exits cleanly after its handoff. The
frontend's normal D-Bus activation starts the installed executable through
systemd; the existing `Restart=on-failure` unit policy stays unchanged. No new
service process is needed while every client is closed.

Opening a frontend connects through D-Bus and calls `PrepareUpdate` against the
same unique owner whose API version was checked. The backend compares its
captured executable device/inode with the current installed file. It never runs
that file as a probe. An unchanged executable is a no-op; unavailable or
non-executable replacements fail without starting an update. CLI status and
configuration reads do not request updates.
The explicit `herd --json backend-update` command provides the same coordinated
path for headless maintenance without changing passive CLI observation.

## Drain and one-shot handoff

An accepted update closes mutation admission atomically with the existing job
lock. Already accepted work remains service-owned and runs to completion.
Read-only observations remain available. The backend waits until its update
reply has been dispatched, then drains work, saves state and closes its D-Bus
connection gracefully. No model unload, router shutdown, transfer pause or
mutation retry is part of this path.

The private `.backend-update.json` beside hosting preferences contains at most
4 MiB: its format identifier, pinned hosting preferences, previous
observations and up to 64 terminal operation results. It contains no executable
commands or unfinished work. This is a bounded operational handoff, not a
command queue or model registry. Atomic creation refuses an existing file. If
preparation fails, the old backend remains alive, reports update failure and
reopens admission. A retry must be requested explicitly.

The replacement validates the handoff format and bounds before serving
requests, and consumes it only after acquiring the exclusive bus name. A
duplicate daemon cannot steal the handoff. Pinned preferences survive the
transition: concurrent configuration edits remain stale rather than silently
switching hosts or hiding competitors. Fresh observations replace saved model
states; configuration and runtime are never mutated while restoring them.

Completed IDs remain queryable after this controlled update. Read-only operation
waiters tolerate transient owner loss for up to ten seconds. Unknown IDs and
unsupported API versions still fail without replay. A later ordinary restart
does not retain results, because the handoff has already been consumed.

## Frontend and acceptance boundaries

Frontends wait up to 120 seconds for the owner to change, then validate the new
owner before continuing. They do not repeat `PrepareUpdate` on acknowledgement
loss or timeout. Accepted updates outlive the requesting window. Clients already
connected use ordinary backend notifications and rereads; they never replay a
pending action merely because the backend changed.

Close old windows and remove the applet during package installation, then open
the installed frontend or re-add the applet after the transaction. Updating
while the package transaction itself is still running is unsupported. A pending
handoff can be consumed by a later compatible installed backend even if another
package update happened while all clients were closed. Private invalid or
unreadable handoff state is not silently ignored.

Backends predating this protocol need a one-time management-service restart to
bootstrap it. No legacy fallback kills an unknown running daemon. Automated
acceptance uses private buses, bounded real backend subprocesses and fake HTTP
runtime state; real user-service restart and native frontend acceptance must be
checked separately without restarting inference.
