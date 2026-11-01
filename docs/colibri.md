# Colibri

Herd can manage an independently installed Colibri text-model engine alongside
llama.cpp. It does not install or build Colibri, drivers or family engines.
Python 3, the native launcher/modules and the chosen family engine must already
be available. The reviewed reference and cross-engine safety boundary are in
[ADR 0008](decisions/0008-engine-integrations.md).

The backend discovers `coli` on its own `PATH`, then the standard user-local
`~/.local/bin` prefix, including support modules in `libexec/colibri`. A first
native setup is not required for catalogue discovery. This does not change the
service environment, install an engine or execute a launcher to discover paths.

## Window workflow

1. Select **Colibri** in the window's engine selector.
2. Open **Find models**, filter the installed native catalogue and review a
   model's repository, immutable commit, license, complete file set and size.
   The native default destination is used unless an explicit directory is given.
3. Confirm **Download**. Follow native byte progress and verification in
   **Downloads**; pause explicitly and review before resuming. Closing the
   window does not own the transfer. A completed download never loads a model.
4. Choose **Load**. The service verifies files, selects the model in native
   configuration and launches its owned host in one explicit operation.
5. Use **Settings** for fine control. An existing configuration's parameters and
   credentials are preserved. First configuration uses CPU settings and loopback
   port 8000; GPU/resource tuning remains available in the drawer.

The Models page can read native catalogue destinations and the currently
selected custom directory through bounded native manifests. These observations
establish file sizes, not compatibility or integrity. The last acquisition plan
is restored after reopening the window. Other custom directories can be reviewed
explicitly; Herd does not create a parallel editable library database.

Models shows local artifacts and the selected instance. Find models shows only
catalogue results; Downloads shows only transfer state, progress and
pause/resume/load actions. The destination is edited in the download
confirmation, not a persistent form repeated across pages. Changing it requires
a fresh scope review before Download can be submitted. Engine configuration is
available in Runtimes without duplicating the catalogue or management-window
transfer controls.

A model already running in another connected engine must be stopped explicitly
before loading a competitor. An external Colibri listener is not adopted and has
no stop action. A recorded owned host can still be stopped when native setup was
edited or corrupted. The panel offers the same owned-instance and transfer
controls through the shared service.

## Fine settings

The native drawer covers explicit context/KV capacity, output limits, GPU device
selection, RAM/VRAM and expert cache budgets, native routing/sampling/reasoning
options, request queue limits and reviewed CPU/GPU environment controls. The
native key names are retained; they are not translations of llama.cpp settings.
Empty fields restore native defaults. Native CLI/environment precedence and
family-specific semantics still apply.

Saving is an atomic, revision-checked edit of `setup.json`, not a restart. Exact
private backups are retained. Unrelated fields, endpoint/model selection and
authentication survive resource edits; credentials are never returned to the
form. External changes are observed without restarting the window. A stale
dirty draft cannot overwrite a newer native revision. Dirty-close confirmation
protects drafts, including when switching engines or closing the application.

Colibri has a single selected native setup, not llama.cpp's INI profile library.
The drawer edits that selected setup. Switching families retains compatible
generic settings; family-specific environment settings require explicit
remapping rather than silent removal. Unsupported native keys fail closed.

## Acquisition contract

- The installed `setup_catalog`, `family_registry` and `setup_download` APIs
  supply choices, engine identities, transfers, partial files and digest checks.
  The embedded caller never invokes `coli setup`, installs an engine, runs a
  converter, benchmarks or opens a browser.
- Inspection resolves a public Hub repository to a 40-character commit and lists
  that immutable revision. Confirmation binds that revision, destination, file
  set, digest metadata and total advertised bytes. Changed scope requires
  review.
- Metadata responses and local JSON have explicit bounds. Up to 4,096 files are
  admitted with safe relative paths. Downloads use validated HTTPS to the Hub
  and its CDN domains, with no proxies, insecure TLS, user-supplied endpoints or
  automatic token lookup. Private/gated repositories are not supported.
- LFS files use advertised SHA-256; small git files use their advertised
  git-blob digest. Native verification checks existing files before reuse and
  acquired files before selection/loading. Size-only presence never establishes
  integrity. A corrupt existing final file fails without overwriting it or
  another model.
- Colibri owns `.part` files and transfer/resume behavior. An explicit pause
  addresses only the identity-checked acquisition host, never inference.
  An uncertain worker outcome blocks automatic resume/new loads.
- The host executes a private, atomically written Python worker file. Python
  source never passes through Process Compose manifest environment substitution.
  Native import and pre-transfer validation failures publish a sanitized
  terminal failure for the reviewed scope instead of leaving it in `starting`.
  A matching completed result remains valid after the one-shot host exits;
  a missing host alone never proves completion or permits automatic resumption.
- Advertised-size and free-space admission are not hard disk/bandwidth quotas.
  Native transfers may retry reads. Verification of large checkpoints can take
  substantial time and I/O; the UI does not perform it on its rendering
  executor.
- Models requiring native post-install conversion/preparation are marked. Herd
  can download their files but does not run those steps or pretend they are
  loadable. An unavailable family engine is also shown rather than installed.

The private host directory, acquisition scope/progress and integrity receipts
live under the Herd configuration directory. They describe accepted operations
and file integrity, not a second native catalogue or model configuration source.
Native `setup.json` remains at `COLI_SETUP_HOME`, then the XDG data/home
fallback. The backend's environment determines these locations, not environment
variables set only on an already-connected CLI.

## CLI

The non-interactive CLI returns the same JSON envelope as other Herd commands.
Use an ID returned by the installed catalogue in place of `example-model`:

```sh
herd --json runtimes
herd --json colibri-catalogue
herd colibri-plan example-model > plan.json
herd --json colibri-download plan.json --max-bytes 32000000000 --yes
herd --json colibri-pause
herd --json start --runtime colibri
herd --json stop --runtime colibri --yes
```

The plan-file example deliberately omits `--json`: the ordinary pretty output is
the raw plan accepted by `colibri-download`, not an envelope containing it.
`colibri-load plan.json --revision HASH` explicitly selects and starts a
reviewed artifact; `colibri-select` selects without starting. Omit the revision
only when creating a first native setup; otherwise use the revision from
`runtimes`. `start --runtime colibri` starts the already selected setup. Exact
runtime IDs may replace the engine selector. Colibri launch/download operation
success means host acceptance; poll `status`/`runtimes` for subsequent
instance/transfer state.

## Unverified properties

Automated native API doubles and HTTP/UDS/subprocess fixtures are not live
Colibri acceptance. Actual model loading, GPU behavior, live Hub transfer,
independent scope survival, desktop frames and input need separate
owner-authorized tests. Gateway identity/readiness is not inference quality or
absence of active requests. Only connected native sources are covered; arbitrary
external processes or other ports are not discovered or treated as machine-wide
GPU reservations.
