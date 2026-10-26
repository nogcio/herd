# Using Herd

## Configure an existing runtime

Install llama.cpp separately, then open Herd. With `llama-server` and
`process-compose` on `PATH`, the management window checks executables, creates
missing private preferences and starts the empty router automatically. Existing
preferences and valid native presets are retained. Runtime setup remains
available for nonstandard executable paths or failed checks. Alternatively
create private `~/.config/herd/server.json` and `models.ini` using
[the templates](../templates/server.json) and
[native configuration](presets.md). Use existing GGUF paths; registration
neither moves nor redownloads weights. `HERD_CONFIG` can select another absolute
hosting-preferences path for an isolated backend process. The variable must be
set on the service, not just a client shell. No global AI or desktop
configuration is modified.

The reference contract was reviewed against llama.cpp b11856 / `3642238cc` and
Process Compose v1.122.0, but versions are not pinned or compared during
admission. Executable probes must succeed; incompatible command flags or API
responses still fail. CUDA builds may need explicit shared-library directories
in `server.json`. All clients require session D-Bus activation registration.

Automatic llama.cpp configuration is unsupported: both
`/etc/llama.cpp/config.ini` and the selected host's
`$XDG_CONFIG_HOME/llama.cpp/config.ini` (otherwise
`$HOME/.config/llama.cpp/config.ini`) must be absent and inspectable. Herd
checks before executable version probes and router launch; it does not read
their contents, remove them or relocate HOME/cache. Rendered Process Compose
path/environment values containing `{{` or `}}` are also rejected because its
template-disable setting covers commands only.

The ready host records its cache root, both automatic-config paths, endpoint and
server path in private operational metadata. Later reload/load/download
admission rechecks recorded paths even if the backend environment changed.
Hosts without matching metadata cannot admit new starts or cache operations;
existing inference remains running. Inspect current state and explicitly
authorize router replacement to establish identity, rather than relying on a
backend restart. Herd never automatically stops/restarts the router for this.
Independent creation of configuration after the last check remains an
unsupported filesystem race, not an atomic isolation guarantee.

```sh
cargo run --locked --bin herd -- --json config
cargo run --locked --bin herd -- --json server-up
```

The router starts without a model or login autostart. Closing the terminal that
issued `server-up` does not own its lifetime; Process Compose daemonizes it.
Logs and private IPC live under the configured state directory. Inference runs
offline; missing files fail rather than trigger an implicit download. Explicit
acquisition uses separate native download workers. There are no
application-managed cgroup memory caps or GUI runtime downloads. Process Compose
is installed as a package dependency; llama.cpp is installed separately.

## Management window

```sh
cargo run --locked --features applet --bin herd-cosmic
```

The window shows native profiles and runtime state and requests empty-router
startup once. It retains a responsive configured host without restarting it,
even with a running model. If startup fails, inspect the error and choose
**Start router** to retry. Timers, file changes and backend reconnection do not
retry startup. Invalid existing configuration requires an explicit startup retry
after repair. Starting the router is not loading a model. Choose **Load** on a
model to load its local weights. Confirm **Unload** before switching to a
competing model; active requests may be interrupted. There is no automatic idle
detection or eviction.

Select a preset row or **Settings** in its overflow menu to open the native
right-hand context drawer for that model. Selection does not load it. Its
fields cover every [supported native option](presets.md), including GPU/MoE
placement, context/batching, KV cache, CPU threads, prompt/reasoning, sampling
and lifecycle. Values remain literal: advanced options are not guessed, rounded
or translated to restrictive sliders. All sections share one scrolling drawer;
click a section heading to expand or collapse it independently. Known
boolean/enum values use native controls; other values retain literal input.
Reset removes only the model override, not an inherited `[*]` setting. Startup
loading remains disabled by policy.

**Save** submits the complete explicit option set in one backend-owned,
revision-guarded atomic save, retaining a preset backup and all unrelated file
bytes. It never restarts or reconfigures running inference. Concurrent changes
reject the save without losing the draft. **Discard** restores the last-read
draft values without writing. The drawer's **Reload settings** icon reads a
fresh snapshot and asks before discarding unsaved work. Closing a dirty drawer
also requires explicit discard. Changing pages or selecting another model
retains its draft; reopening Settings restores it without replacing unsaved
values.

The left sidebar separates **Models**, **Find models**, and **Downloads**. The
library supports name/ID filtering and reversible name sorting. The header's
**Add model → Import GGUF…** action selects an existing GGUF, suggests an
editable preset ID and requires explicit registration. The backend checks the
regular file and GGUF v2/v3 header, rejects duplicate paths/IDs and stale
configuration, retains an exact preset backup, and atomically appends a profile.
It does not copy or load weights. Header admission is not a compatibility or
completeness check.

Manual edits and newly added sections appear without restarting the UI. Atomic
editor saves are observed by watching parent directories. An invalid or deleted
file is a visible error and blocks new starts, not an empty catalogue. Restore
or correct it; never rely on previously displayed settings to admit a new start.

**Find models** offers public Hub search, repository/variant selection and
explicit native acquisition in a confirmation dialog. Progress appears in
**Downloads** section and popup; pause keeps partial files and resume requires
review. Downloads lists native active/incomplete transfers, not persistent
history. Completed cache entries appear in Models; acquisition does not prove
readiness. No raw download URLs are displayed. See [acquisition](downloads.md)
for supported files, metadata-size ceilings and post-transfer integrity checks.
A missing local path or load action never authorizes a download.

**Remove…** in a model's overflow menu first reviews its scope. Removing a
profile keeps the original weights; deleting a native cache entry removes its
cached/partial files through llama.cpp. Both require confirmation and never
automatically stop inference or a transfer. See
[removal semantics](downloads.md#removing-a-model).

The application and applet negotiate the desktop locale at startup using Fluent,
with embedded English fallback and Russian translations. Native option keys,
model identities, paths, enum literals and CLI JSON stay unchanged. Technical
diagnostics from the backend/runtime remain English. Restart the application
after changing its locale environment; no system language setting is modified.

## Run in the panel

Install the [Arch package](installation.md) first, then open panel settings:

```sh
cosmic-settings panel
```

Add **Herd** using panel settings. Registration does not
rearrange the panel or launch inference. Direct execution of
`herd-cosmic-applet` does not place the applet in the panel. It is not
a StatusNotifierItem tray icon.

Both panel orientations use a symbolic icon, with warning/download variants for
errors and active transfers. The popup lists model states, explicit load/unload
actions and transfer progress. It does not monitor GPU utilization or memory.
**Manage models…** launches or activates the single-instance application.
Application, panel and CLI share backend observations and operation admission,
not in-process UI objects. Resume opens explicit acquisition review in the app.

Closing a popup/window never unloads a model or stops the host. Native
placement, focus, input, scaling and rendering require interactive acceptance;
process startup or successful widget construction does not establish those
properties.

## CLI and automation

Use `cargo run --locked --bin herd --` in place of the
executable below when running from source:

```sh
herd --json status
herd --json start example-model
herd --json stop example-model --yes
herd --json config
herd --json import example /models/a.gguf --revision TOKEN
herd --json set example-model ctx-size 8192 --revision TOKEN
herd --json server-down --yes
```

Obtain `TOKEN` from `config` immediately before an edit. Changes to comments
also change the revision. `server-down --yes` authorizes interrupting all router
models.

Mutations return service-owned operation IDs. The CLI prints an accepted ID on
stderr immediately and waits for the outcome; `operation ID` retrieves it from
any client. Closing the CLI does not cancel it. `status` returns the common
backend report, including observation errors and bounded operation history.
Only CLI output uses JSON; D-Bus transports native typed values.

History retains at most 64 operations in memory. After backend restart an old
ID is unknown; inspect current runtime state before explicitly retrying. The
service never automatically replays commands or unloads models at shutdown.
Manage the backend with `systemctl --user status herd.service`.
`restart` is not a model restart, and `server-down --yes` remains the explicit
router/model shutdown operation.

Admitted JSON commands emit one stdout object with `schema_version: 1`, `ok`,
and either `data` or `error`. Errors currently use `code: operation_failed` with
safe English context. Exit codes are 0 for success, 1 for operational failure,
and 2 for invalid arguments/missing confirmation flags. Clap help and argument
errors precede configuration, desktop and network access. There are no
prompts.

`start` waits for router-reported loaded state. `stop` waits for an unloaded
state. `server-up` waits for management readiness without loading weights.
Failures/timeouts do not automatically stop a model; inspect status before
retry.

Read-only model diagnostics are also available:

```sh
cargo run --locked --bin herd-status
```

This does not start/stop inference or send generation requests. Decorative
artwork is loaded and cached by the GUI without a separate prefetch command.
