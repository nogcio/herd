# ADR 0005: Distribution-owned installation

Status: accepted; AUR recipe prepared, not submitted.

Use a normal `herd-git` PKGBUILD and generated `.SRCINFO` for Arch installation
and updates. Pacman owns package assets. Do not maintain custom transaction
installers, prefix installers or development-registration commands as parallel
deployment paths. Direct Cargo commands remain sufficient
for builds; xtask retains source checks only.

Install the shared backend as a user unit, not a system service. Do not add
package hooks that start/stop/restart a backend or inference, enable autostart,
change panel layout, or mutate user configuration. Presets, weights and state
remain outside package ownership. Users explicitly coordinate upgrades with
their active clients and operations.

Arch's standard hook reloads user-manager unit metadata. After the transaction,
frontend startup requests a [coordinated backend update](0007-backend-update.md)
through the existing management API. Systemd owns restarting that service;
neither package scripts nor frontends launch replacement daemon processes.

Package source licensing remains distinct from combined binary obligations.
Use upstream license aggregation plus the reviewed missing-metadata texts; keep
the locked source revision and corresponding-source obligations reviewable.

The recipe uses the existing repository URL. Initial source publication, AUR
submission and installation are separate authorized actions. Previous user-local
assets are not deleted by packaging; their reviewed cleanup must be separately
approved. See [installation](../installation.md).
