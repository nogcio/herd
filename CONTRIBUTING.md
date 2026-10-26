# Contributing

Herd is an early-development local llama.cpp model manager with a COSMIC
frontend. Discuss new architectural boundaries, runtime authority, and
distribution changes first. Keep source, fallback UI resources, documentation
and diagnostics in English. Target-language Fluent resources and desktop
translations are welcome; native option keys, values and CLI machine output
must not be translated.

## Prerequisites

- Git, rustup, and a C linker/toolchain.
- The Rust version/components in `rust-toolchain.toml`; commit `Cargo.lock`.
- Native libcosmic development libraries and `pkg-config`.
- `dbus-daemon` for isolated IPC tests; no running desktop bus is required.
- `desktop-file-validate`/`desktop-file-install` and `appstreamcli` for native
  metadata.

On Arch, package-build dependencies are declared in
[`packaging/aur/PKGBUILD`](packaging/aur/PKGBUILD). Relevant development
packages include `base-devel`, `pkgconf`, `fontconfig`, `freetype2`,
`libxkbcommon`, `wayland`, `desktop-file-utils`, and `appstream`. On
Debian/Ubuntu, install the corresponding development packages; the exact CI
package list lives in [the workflow](.github/workflows/ci.yml). Review package
operations yourself; project build/check commands do not elevate privileges or
install system packages.

The pinned compiler is the validation baseline. `Cargo.toml`'s Rust 1.93 floor
is not an independently tested minimum-supported-version guarantee. Tests and
builds need neither GPU hardware nor CUDA Toolkit. Native GPU/context preset
parameters are independent of the application's rendering backend.

### Repository tools

Install these reviewed versions explicitly, or use their upstream prebuilt
tools:

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo install rumdl --version 0.3.0 --locked
cargo install lychee --version 0.24.2 --locked
```

Use Gitleaks **8.30.1** from its
[upstream release](https://github.com/gitleaks/gitleaks/releases/tag/v8.30.1).
CI verifies the published Linux archive checksum before extraction; see
[tool/version policy](docs/quality.md). Tooling is not a runtime dependency, and
checks never install tools automatically. Review upstream tool upgrades and
their lockfile warnings separately from this application's dependency graph.

## Checks and test ownership

```sh
cargo xtask check
cargo xtask audit
cargo xtask secrets
```

`cargo xtask check` runs formatting, isolated core/applet lint/tests/rustdoc,
desktop/AppStream validation, Markdown lint, and offline local links.
`cargo xtask audit` checks dependency advisories, declared/discovered licenses,
and approved sources. `cargo xtask secrets`
scans the Git source inventory, including history when present. See
[quality policy](docs/quality.md) and [testing](docs/testing.md).

Add a failure-path test for a bug fix and demonstrate the relevant unchanged
state, not merely `is_err()`. Identify the production owner and smallest
sufficient environment first. Keep private invariants in their module; public
executable contracts belong in `tests/`. Shared fixtures are support, not
another test suite. Do not add a general fixture framework or duplicate upstream
tool tests.

Native UI changes also need interactive COSMIC acceptance. Record actual input,
theme/scaling, and observed frames; a successful process launch or
widget-construction test does not establish layout or interaction quality. Do
not stop an active model to test the UI. Use fake backends for automated
lifecycle tests.

## Change and review workflow

1. Read [AGENTS.md](AGENTS.md), the architecture, and the affected contract.
2. Keep changes focused; reuse existing mechanisms and maintained libraries.
3. Change tests and current documentation together. A cross-cutting decision
   gets an ADR in `docs/decisions/`; do not retain duplicate authoritative
   descriptions.
4. Report exact commands, results, skipped checks, platform, and remaining
   risks.
5. Request human review. Commits, pushes, release/tag creation, repository
   permission changes, and publishing require explicit owner approval.

Do not lower checks, broaden security limits, silence failures, or delete
assertions just to obtain a green result. Preserve unrelated changes. Prefer
direct refactors over compatibility shims that retain obsolete paths. Review
dependency/license/API changes deliberately; don't turn a small applet into a
framework or workspace DAG.

### AI-assisted contributions

Read the project-scoped instructions. AI assistance does not replace source
review, tests, licensing checks, or human accountability; disclose material
assistance in the PR. Check APIs against the exact versions/Git sources in
`Cargo.lock`, not a model's memory or an unrelated latest release. No paid
search tool, MCP, personal agent configuration, API key, or private service is
required to contribute.

### Documentation and evidence

Documentation states current behavior, contracts, provenance, limitations, or
explicitly labeled plans. Do not commit session transcripts, machine
inventories, personal paths, backup locations, or local test-run histories.
Store generated logs and screenshots under ignored `target/`; publish only
deliberately reviewed, redacted artifacts. A screenshot must be real,
attributable, and free of unrelated desktop/private content. Do not claim CI has
passed before a hosted run exists.

## Community, security, and licensing

Follow the [Code of Conduct](CODE_OF_CONDUCT.md). Report sensitive issues
through a verified private channel as described in [SECURITY.md](SECURITY.md),
not public issue logs. Contributions are submitted under this project's MIT
license; you must have the right to contribute code, text, and assets.
Third-party material does not become MIT merely by appearing in a dependency or
screenshot.
