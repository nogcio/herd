# Herd

**Manage local llama.cpp models from a native COSMIC app, a compact panel
applet, or the command line.**

Herd gives you one place to see your models, choose what to load, find new
GGUFs, and edit runtime settings—without juggling launcher scripts or copying
weights you already have. Keep the management window open for your library and
settings, or use the panel for a quick status check and explicit load/unload
actions.

Herd manages model lifecycles and settings; llama.cpp performs inference. Herd
does not provide a chat interface or implement an inference engine. It is a
standalone libcosmic application, not a COSMIC Settings plugin.

## Project status

**Early development; no stable release.** The management window,
panel applet, shared backend and CLI are implemented in this source tree. An
Arch Linux `herd-git` package recipe is included, but is not published in AUR.
This repository is an experimental source preview, not stable binary acceptance.

The target desktop is Linux/COSMIC. Automated tests and offscreen rendering do
not establish native panel placement, input or rendering, real systemd scope
lifetime, or successful live model transfers. Those require separate, authorized
[native acceptance](docs/testing.md#native-acceptance) and human review.

## What you can do

- **Manage your library:** view native presets and router cache entries, filter
  and sort models, and import existing GGUF files without copying or loading
  them.
- **Find and download models:** search public Hugging Face repositories, review
  GGUF variants and sizes, then explicitly download through llama.cpp's native
  cache. Follow progress in **Downloads** or the panel; pause and review a
  resume.
- **Edit model settings:** use the native settings drawer for context, GPU/CPU,
  cache, sampling and other supported preset options. Saves detect concurrent
  edits; manual file changes appear without restarting the UI.
- **Stay in control:** explicitly start the router, load or unload models,
  remove a preset while keeping its weights, or confirm deletion of an idle
  cache entry.
- **Use the same controls from scripts:** the non-interactive CLI returns JSON
  and shares the application's observations and operation policy.

The GUI includes English and Russian localization, optional model artwork, and
the Gather application icon. Herd does not monitor GPU utilization or memory;
native GPU placement and context settings remain available in the preset editor.

### Deliberate boundaries

Opening Herd, checking status, selecting a model, or saving settings never
starts inference or downloads weights. Normal loads run offline: missing weights
fail rather than trigger a download. Download completion never loads a model.

Hosting requires automatic llama.cpp configuration to be absent at the system
and exact host-user locations. Existing hosts without recorded host identity
cannot admit new model starts or cache operations; inference already running is
left alone. Establishing identity requires an explicitly authorized router
replacement, never an automatic restart. These checks are not atomic isolation
from external filesystem writers. See
[hosting constraints](docs/architecture.md#automatic-native-configuration).

Herd does not automatically evict a competing model. Unload it explicitly before
switching; unloading can interrupt active requests. Settings edits do not
restart running inference. Closing a window, popup or CLI does not stop an
accepted operation, model or transfer.

Acquisition currently supports unambiguous
**single-file GGUFs with advertised SHA-256 metadata**, not split models,
sidecars, remote presets or private/gated repositories. The supported native API
downloads current `main`, not a pinned revision; advertised-size admission is
not a hard transfer quota. Acquired files must pass size, header and SHA-256
verification before loading. See
[download limits and removal semantics](docs/downloads.md). Importing or
downloading a GGUF does not by itself establish model compatibility or
readiness.

## Build, install and open

Herd requires a session D-Bus and systemd user manager. Its supported external
runtime versions are **llama.cpp b11856 / `3642238cc`** and **Process Compose
v1.122.0**. Herd checks those versions; arbitrary newer builds are not
automatically compatible. It does not install the runtime, CUDA or Process
Compose for you.

To build the current source without installing or launching Herd, first review
the [development prerequisites](CONTRIBUTING.md#prerequisites), then run:

```sh
cargo build --locked --features applet --bins
```

A Cargo build alone does **not** install the desktop entries or backend
activation registrations required by the clients. The prepared
[Arch installation workflow](docs/installation.md) explains package
prerequisites, installation and updates. Review the declared build dependencies
and build the package from the repository's default branch as a normal user:

```sh
cd packaging/aur
makepkg --cleanbuild
```

After reviewing and installing the package through your package manager:

1. Open **Herd** from the application menu, or run `herd-cosmic`.
2. Select the already installed runtime executables in first-run setup. Setup
   checks their versions and creates missing configuration without overwriting
   existing files.
3. Import a local GGUF or review a download. Choose **Start router**, then
   explicitly **Load** a model when ready.

For the optional panel companion, run `cosmic-settings panel` and add **Herd**.
Directly running `herd-cosmic-applet` does not place it in the panel.
**Manage models…** opens or activates the standalone window. Installation does
not enable login autostart or change your panel layout. Pacman owns application
installation and updates; preferences, presets and weights remain user data.

See the [user guide](docs/usage.md) for setup, settings and CLI details, or
[native presets](docs/presets.md) for manual configuration.

## CLI

With the backend activation registrations installed:

```sh
herd --json status
herd --json config
```

To explicitly start the empty router, load a configured model, then unload it:

```sh
herd --json server-up
herd --json start example-model
herd --json stop example-model --yes
```

The CLI never prompts interactively. Destructive actions require explicit flags;
download and removal commands require a reviewed plan. See
[CLI and automation](docs/usage.md#cli-and-automation) and
[acquisition commands](docs/downloads.md#cli).

## How it fits together

The window, applet and CLI use one typed session D-Bus API, owned by an
on-demand systemd **user** service. Native llama.cpp INI presets remain the
model configuration source; Herd does not keep a second model registry. Process
Compose hosts the loopback llama.cpp router in an independent user scope, not
inside the backend service's cgroup. See the
[architecture contract](docs/architecture.md) for ownership and safety limits.

## Documentation and contributing

- [User guide](docs/usage.md) · [Installation and updates](docs/installation.md)
- [Native configuration](docs/presets.md) · [Downloads](docs/downloads.md)
- [Linux/COSMIC integration](docs/platform.md) ·
  [Artwork provenance](docs/icons.md)
- [Contributing and checks](CONTRIBUTING.md) ·
  [Testing and acceptance](docs/testing.md)
- [Security](SECURITY.md) · [Contributor instructions](AGENTS.md)

## License

Herd-authored source is [MIT licensed](LICENSE). The combined
application/package includes GPL-covered `cosmic-panel-config` and is identified
as **GPL-3.0-only**, not an MIT-only binary. Dependencies retain their own
license, notice and source obligations. Review the
[third-party distribution requirements](docs/third-party.md#license-admission-and-distribution)
before distributing binaries; corresponding-source delivery is not automated.
