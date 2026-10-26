# Testing and acceptance

## Commands

```sh
cargo xtask check
cargo xtask audit
cargo xtask secrets
```

Install the pinned [repository tools](../CONTRIBUTING.md#repository-tools)
first. The thin xtask delegates checks to Cargo, Clippy, rustdoc, rumdl, Lychee,
cargo-deny, Gitleaks, desktop-file-validate and appstreamcli. It stops at a
failed stage and does not install global tools or remove failures.

Use `cargo xtask check --scope core` or `--scope applet` for a feature scope,
`cargo xtask docs` for Markdown/offline links, and `cargo xtask validate` for
native desktop metadata. Isolated tests need no GPU, public model host,
inference server, desktop session, credentials, or real user configuration. The
IPC integration test requires the installed `dbus-daemon` executable and creates
its own bounded private bus/service subprocesses, not user-session units. Its
explicit bus configuration excludes real user activation directories, so an
installed Herd cannot be activated by the test's startup race.

## Ownership

| Contract | Owning tests |
| --- | --- |
| Native defaults, fail-closed validation, conflict-checked edits | `config` |
| Atomic complete edits, inherited reset, unchanged comments/competitors | `config`, private-bus IPC test |
| Atomic save/delete/recreation notifications | `config`, real temporary directory |
| Passive observation, fresh load admission, unchanged competitor | `controller`, isolated HTTP mock server |
| Quoted argv, offline/autoload policy and cross-process lock primitive | `hosting` |
| Public metadata, file selection, corrupt artifact rejection | `downloads`, isolated files and HTTP mocks |
| Download admission, cache reuse, pause races, scoped removal | `controller`, isolated HTTP mocks |
| Native SSE progress, bounded reads, unchanged competitors | `runtime`, `backend::service` |
| Download wait timeout without cancellation | `backend`, private bus |
| Stale search completions and download browser construction | `ui::downloads` |
| Artwork transport, decoder and cache limits | `icons` |
| State/action formatting and inactive-ID progress rejection | `ui::view`, `backend::service` |
| Sibling app launch, activation environment, unchanged pending operation | `ui::manager` |
| Complete form coverage, draft/reload races, dirty drawer close, stable field dependencies | `ui::editor` |
| All-expanded layout bounds and clipped raster output with CPU fallback | `ui::editor`, real headless renderer |
| Page changes retain drafts and operations; library/transfer filtering | `ui::navigation`, `ui::manager` |
| Russian negotiation, English fallback and stale-state warning | `ui::localization` |
| Private-bus singleton, typed signals, disconnected client, restart without replay | `tests/backend.rs` |
| View construction and async artwork state | `ui::view`, `ui::icons` |
| Narrow/wide library and transfer row layout bounds with CPU fallback | `ui::view`, real headless renderer |
| Help, invalid arguments, JSON failure and confirmation admission | `tests/cli.rs`, bounded real subprocesses |
| Native comment/section ambiguity, special files and preset symlink mutation | `config`, `regular_file` |
| Native cache repository/revision scope, ambiguity and receipt integrity | `runtime::cache`, `controller` |
| Same-owner API version validation and owner loss without replay | `backend`, private bus |
| Window/application exit guards and canceled removal-plan generations | `ui::editor`, `ui::manager` |
| Automatic-config presence/inspection failures before probes and launch | `hosting`, isolated paths and bounded fake executables |
| Frozen host config/cache identity and child-dispatch admission | `hosting`, `controller`, isolated files/HTTP mocks |
| Rendered Process Compose template-delimiter rejection | `hosting`, bounded fake executables |
| Unpinned executable probes and unchanged weights/competitors | `hosting`, bounded fake executables |
| One-shot window setup/startup; passive panel and failed-start non-replay | `ui::setup` |
| Shared concurrent startup and responsive-host preservation | `backend::service`, `controller` |
| Coordinated update drain, failed handoff, restored results and unchanged competitors/transfers | `tests/backend.rs`, private bus and HTTP mocks |
| Executable identity, private bounded one-shot handoff and terminal-only results | `backend::update` |
| Update owner replacement, version validation and read-only waiter reconnection | `backend`, private bus |

Keep private invariants in their owning module. Fixtures support production
contracts; they are not another test suite. Do not duplicate generic upstream
tests, widen visibility for fixtures, or install test data into real services.
Bug fixes need relevant failure-path and unchanged-state assertions.

Test product behavior, not repository file layouts, command declarations or
the checks themselves. Packaging acceptance uses actual optimized builds and
an explicit package installation/upgrade, not a second installer implemented
inside tests.

## Native acceptance

Use the [panel-hosted route](usage.md#run-in-the-panel) for
applet acceptance and `herd-cosmic` for the standalone window.
Compilation, a PID, widget construction and an HTTP response establish different
things; none is a substitute for actual input and rendered frames.

- Check horizontal/vertical panel orientation, popup anchoring, hover/focus,
  scaling, scrolling, keyboard navigation, light/dark theme and artwork
  fallback.
- Open **Manage models…** twice: only one management window should remain.
  Check its header actions, resizing, focus and stop confirmation.
- Check native sidebar navigation and the right-hand settings context drawer at
  wide/narrow widths. Check all parameter categories, Reset, dirty-close/reload
  dialogs and draft retention across model/page changes. Closing the drawer
  must not cancel accepted work. Test keyboard focus, enum popups and long paths
  in both English and Russian; neither compilation nor dependency-cache tests
  establish frame times or perceived scrolling/input latency.
- Cancel a stop: no unload request should occur. Actual load/unload tests
  require owner-authorized models or disposable fixtures, never an active user's
  work.
- Check all three sidebar sections, library filter/sort, Add model menu, local
  import, Enter/clear search, repository Back, variant selection,
  download Cancel/Escape and full-width rows at narrow widths. Do not resume a
  real transfer merely to test navigation. Progress tests without a live
  transfer establish transport/state handling, not live bandwidth or visual
  acceptance.
- With separately installed runtimes and absent preferences, open the window:
  detect executables, initialize missing settings and start an empty router.
  Reopen against a running model: no reload, unload or restart. Missing binaries
  must show setup; failed startup must remain actionable without timer/reconnect
  retries. These live host checks require explicit authorization.
- Check pending resume review feedback and leave/return before its completion:
  a late result must not reopen a dialog. Escape from dirty-close/reload keeps
  values and cancels pending window close. Check populated setup labels and
  compact popup Resume rows, not only empty forms and idle inference rows.
- Review both removal dialogs. Cancel must leave the model unchanged. Actual
  deletion acceptance needs explicitly authorized disposable cache entries;
  never use existing personal model weights as a removal fixture.
- Edit a preset manually and by atomic save, add/remove a model, corrupt/restore
  the file, and inspect pending/error states without restarting the UI.
- A running model must not be restarted by a watcher or settings save. Verify a
  competing start fails with the original instance unchanged.
- Close/reopen the popup/window during an operation. Inference must remain
  independent of UI lifetime. Test host shutdown only with authorization.
- Verify D-Bus activation through the installed user unit. With explicitly
  authorized disposable inference, verify that backend stop/restart leaves the
  separate router scope, model and transfer unchanged. Private-bus tests do not
  establish real systemd cgroup behavior or panel input/rendering.

## Evidence

Headless CPU fixture renders demonstrate only their fixture state, layout and
raster output. They are not live COSMIC window/panel screenshots, do not
exercise real input or compositor behavior, and cannot establish native
acceptance.
Automatic-config tests establish fail-closed checks, not atomic protection
against independent filesystem writers creating configuration after admission.
Source changes also do not update an installed package: acceptance must identify
the actual build being exercised rather than assume installed clients match it.

Manual acquisition acceptance must inspect a small public GGUF, explicitly
authorize transfer, observe window/panel progress, pause/resume, verify
completion, and close UI without stopping the native worker. Automated tests
download no weights. A mocked success response does not establish a real
transfer result.

Keep local logs/screenshots/reports under ignored `target/`. Record exact
commands, results, skipped checks and remaining limitations. Do not publish raw
unit text, environments, private paths, credentials, prompts or unrelated
desktop contents. An earlier result does not establish acceptance of a later
revision. Human review is required even when automated checks pass.
