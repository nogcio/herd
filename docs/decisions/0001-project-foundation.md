# ADR 0001: A small, contract-owned native utility

Status: accepted.

Keep one runtime Rust package, feature-gated COSMIC presentation, a headless
CLI, and ordinary modules with clear responsibilities. Keep repository checks in
the separate thin xtask; Arch packaging owns installation. Do not add
speculative crates, compatibility layers, plugin systems, or a generic backend
framework.

The native runtime owns inference and its model children; a maintained host owns
the router daemon. Application code owns validation, explicit action admission,
configuration editing, and presentation. See
[runtime direction](0002-model-management-direction.md) and the
[current architecture](../architecture.md).

Use standard Cargo tests and existing quality tools. Tests follow production
contracts with bounded subprocesses, isolated files and fake HTTP services.
Actual desktop input and frames are required for native acceptance; compilation,
widget construction, a PID and an HTTP response prove different things.

Public documentation describes current behavior and explicitly marked limits.
Local verification history, private paths and screenshots stay under ignored
`target/`. Human review, dependency licensing and publication authorization are
separate obligations. No source check grants permission to publish artifacts.
