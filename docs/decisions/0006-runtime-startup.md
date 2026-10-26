# ADR 0006: Installed runtimes and management-window startup

Status: implemented in source; package and native acceptance remain separate.

Users install llama.cpp independently, choosing a CPU, CUDA or Vulkan package.
Herd depends on the distribution's Process Compose package rather than bundling
or downloading runtime executables. Neither dependency is pinned by runtime
version: successful executable probes are required, not particular version
strings. The reviewed reference remains llama.cpp b11856 / `3642238cc` and
Process Compose v1.122.0. Arbitrary other versions are accepted but not
certified to preserve that contract's flags, API semantics or offline/autoload
policy. Unsupported responses fail; no alternate inference transport is
introduced.

Opening the management window detects executables on `PATH` and, if hosting
preferences are absent, submits the existing service-owned setup operation.
Setup validates existing native presets, checks executables and creates only
missing files. Existing hosting preferences are never rewritten by startup.
Failed discovery/checks leave the runtime setup page available without repeated
automatic attempts or GUI downloads. Invalid existing configuration consumes
automatic startup for that window: repairing or deleting it later does not
authorize a watcher-driven start or initialization; use the explicit controls.

After a valid configuration observation, the window requests empty-router
startup once per window lifetime. This is an explicit client operation distinct
from passive backend activation, CLI status, panel observation or file watching.
Concurrent startup callers share the running operation. A responsive router
with matching host identity is retained without reloading presets, unloading
children or restarting inference. Missing/mismatched identity fails without
replacing the host. A failed or uncertain startup is not automatically retried
after refresh, reconnection or configuration changes; a manual retry is offered.

Models and downloads remain explicit. The router retains offline mode, disabled
autoload and eviction, and an independent user scope. Closing the window does
not stop it. This is window startup convenience, not login autostart or generic
runtime supervision.
