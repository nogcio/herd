# Arch Linux installation and updates

## Package workflow

Herd uses the `herd-git` Arch package. Its recipe is
[`packaging/aur/PKGBUILD`](../packaging/aur/PKGBUILD), with generated
[AUR metadata](../packaging/aur/.SRCINFO). There is no custom installer or
package install hook. Pacman owns application files and upgrades; the backend
continues to run as a systemd **user** service.

The recipe is prepared for AUR submission, not published there. It builds from
the Herd source and Cargo.lock on the GitHub repository's default branch.
Publishing source does not constitute a stable binary release or AUR submission.

Review the recipe and build as a normal
user on an up-to-date Arch system:

```sh
cd packaging/aur
makepkg --cleanbuild
```

Install missing build dependencies through your normal package-management
workflow first. They are declared in PKGBUILD; nothing installs them implicitly.
Use pacman or an AUR helper to install the resulting `herd-git` package and to
perform later upgrades. Do not run `makepkg` as root. For explicit interactive
dependency/package installation, the conventional alternative is
`makepkg --syncdeps --install`; project checks never run it.

The package builds all five executables with the `applet` feature and a frozen
Cargo.lock after an explicit dependency-fetch phase. It uses Arch's Rust
compiler, or the user's stable rustup toolchain, rather than installing the
repository's pinned CI compiler. Rust 1.99.1 is the tested baseline. Tests use
isolated fixtures and a private bus; no models are downloaded or started.

Only optimized `target/release` executables are packaged. Cargo's standard
release optimization is used without CPU-specific flags, so the package is not
tied to the build machine's processor. Build-time license aggregation runs
offline after preparation has fetched the locked dependencies.

The recipe disables makepkg's GCC LTO injection because Rust's LLVM linker
cannot consume ring's GCC slim-LTO objects. This does not disable Cargo's
release optimization or change the runtime's safety checks.

## Installed files and configuration

- Executables: `/usr/bin/herd`, `herd-service`, `herd-status`, `herd-cosmic`
  and `herd-cosmic-applet`.
- Application/applet entries: `/usr/share/applications/`; AppStream metadata:
  `/usr/share/metainfo/`.
- Backend: `/usr/lib/systemd/user/herd.service` and session D-Bus activation
  under `/usr/share/dbus-1/services/`, referencing `/usr/bin/herd-service`.
- License bundle, supplemental upstream texts and obligations:
  `/usr/share/licenses/herd-git/`. User/developer documentation remains in the
  source repository rather than installing incomplete copies with broken links.

No system service, root helper, global configuration file, autostart enablement,
panel-layout modification or model operation is installed. There is no pacman
hook that restarts a backend, router or model. Distribution-provided metadata
refresh hooks may still run as part of the normal package transaction.

Preferences in `XDG_CONFIG_HOME/herd`, native presets, existing model weights,
cache and private runtime state are not package assets. Upgrades/removal do not
rewrite or delete them. Install a llama.cpp package separately before opening
Herd; CPU, CUDA and Vulkan variants are alternatives, not simultaneous
requirements. The package metadata names runtime choices. Process Compose is a
normal Herd dependency from Arch extra. Neither runtime has an exact-version
package constraint or a version-string admission gate. The reviewed reference
builds are llama.cpp b11856 / `3642238cc` and Process Compose v1.122.0;
acceptance of another version does not establish compatible flags or API
behavior.

On first opening the management window, Herd finds `llama-server` and
`process-compose` on `PATH`, checks executable probes and initializes missing
configuration. It reuses existing valid presets and never overwrites hosting
preferences. If discovery or checks fail, select the executables in runtime
setup. Nonstandard CUDA builds may need explicit library directories.
An arbitrary newer runtime is not automatically a supported contract.

Open **Herd** from the application menu. Add its optional applet through
`cosmic-settings panel`. Clients activate the backend on demand; opening a
client never loads a model. The management window requests empty-router startup
once per launch; the panel and CLI do not. Startup errors remain visible and
require an explicit retry. No model is loaded and no weights are downloaded by
first-run setup or window startup.

## Updates and previous user-local installations

Save drafts, close the application and remove its running panel applet before
updating. Wait for accepted backend operations to finish and do not submit new
operations during the package transaction. The package does not police active
sessions or automatically replace a running process. After updating, reload the
user manager's metadata and explicitly restart only the management backend when
it is safe to do so; re-add the panel applet. Never stop the router as an update
step. A logout can affect inference and is not a safe substitute.

Previous user-local executables and desktop/D-Bus/systemd registrations can
shadow `/usr` package files. Pacman deliberately does not remove files from
users' home directories. Review effective registrations and PATH, back up the
exact obsolete application files, and remove only those explicitly approved for
cleanup. Preserve presets, preferences, weights, runtime binaries and existing
installer backups. Live cleanup is separate from building this package.

Pacman provides package-file ownership and cached-package downgrade mechanisms,
not application-data backup or guaranteed compatibility between backend API
versions. Successful package staging does not establish a real install/upgrade
or native panel acceptance. License aggregation assists review but does not
replace the
[distribution obligations](third-party.md#license-admission-and-distribution).
