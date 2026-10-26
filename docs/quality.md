# Quality and CI policy

## Required source checks

Run `cargo xtask check`, `cargo xtask audit`, and `cargo xtask secrets` before
requesting acceptance. The same public entry points are used by
[GitHub CI](../.github/workflows/ci.yml). Checks require no private service, AI
key, GPU workload, or maintainer credentials.

- Formatting, warnings-as-errors Clippy, tests/doctests, and warnings-as-errors
  rustdoc cover isolated core and applet feature routes. `--all-features` alone
  is not evidence that minimal/default compilation works.
- Project unsafe code is forbidden in Cargo lints and independently at Clippy
  invocation. Dependencies may contain unsafe code; this is not a sandbox claim.
- Markdown uses rumdl; Lychee checks local files/fragments offline. External
  links are not continuously certified and must be reviewed when relied upon.
- cargo-deny evaluates fresh advisories, accepted licenses, and approved
  Git/registry sources. Maintenance-only findings may receive owner-approved,
  ID-specific acceptance with rationale and a review date; vulnerabilities are
  not covered by that acceptance. Do not copy another project's exceptions. See
  [dependency obligations](third-party.md).
- Gitleaks directly scans tracked changes, staged content and history with
  redaction. Ignore rules are not secret
  scanning, and a clean scan does not prove the absence of every secret or
  private detail.

Stage new files before the secrets check: untracked files are not part of the
commit inventory and are not scanned until added. Staged content is scanned even
when it differs from the working copy. There is no separate scanner script,
snapshot directory or custom detection policy.

The audit explicitly requests `--workspace --all-features`. The optional
libcosmic package is named `cosmic` in dependencies, matching its Rust crate
and Cargo metadata edge. A package-key/crate-name mismatch can cause the pinned
graph reader to filter GUI dependencies even with all features requested.
Unexpected missing-source warnings are a coverage failure to investigate, not
evidence that the desktop dependency graph is clean.

## Tool and dependency versions

`rust-toolchain.toml` selects Rust 1.99.1 with Clippy/rustfmt. `Cargo.lock` is
committed; normal invocations use `--locked`. Compiler/tool updates are
deliberate changes, not automatic side effects of linting. `Cargo.toml`'s
declared Rust 1.93 floor is not separately tested as the minimum supported
toolchain.

Repository tools: cargo-deny 0.20.2, rumdl 0.3.0, Lychee 0.24.2, Gitleaks
8.30.1. Cargo tools install from their locked upstream releases. Gitleaks' Linux
x64 archive SHA-256 is
`551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb`, as published
in its upstream GitHub release asset metadata. Source actions use reviewed
immutable commits, with version comments for update navigation.

Tool installation is an explicit prerequisite, not a build/check side effect.
Tool dependencies are separate from the application's dependency graph. Review
their upstream lock warnings too; a version pin is not a vulnerability
assessment.

## Hosted boundaries

CI runs untrusted contributions on GitHub-hosted Linux runners via
`pull_request`, not privileged `pull_request_target`. Permissions are read-only;
checkout does not persist credentials. No publishing secrets, environment,
signing, deployment, or OIDC permissions are supplied. Cache restoration never
replaces execution of checks; only default-branch pushes may save shared
dependency caches.

All check groups run on every relevant workflow invocation; there is no custom
path-selector or test scheduler. `CI required` fails closed if any prerequisite
fails, is cancelled, or is skipped. The workflow configures checks; a maintainer
must separately verify hosted results and branch protection after publication.
Managed runner labels and package mirrors are not bit-for-bit OS pins.

No coverage percentage threshold, visual acceptance, macOS/Windows support,
registry publication, or release signing is implied by this workflow. Changes to
the supported platform/features require corresponding evidence and CI review.

## Human review

Review scope, architecture, tests/failure paths, licensing, security impact, and
native evidence. New dependencies need a concrete responsibility and consumer;
reuse ordinary ecosystem mechanisms before inventing parsers or frameworks. Keep
files cohesive; don't minify or arbitrarily fragment code to satisfy a metric.
Policy changes require explicit owner approval and documented risk/obligations,
not a green-badge rationale. Never fabricate evidence or discard failures.
AI review supplements, not replaces, owner approval.
