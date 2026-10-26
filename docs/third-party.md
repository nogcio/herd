# Dependencies, licenses, and source provenance

## Project and artwork

Project-authored source is licensed under [MIT](../LICENSE). Dependencies keep
their own terms. A permissive license on application-authored files does not by
itself determine the obligations of a binary linked with third-party code.

Optional Qwen/Recursive artwork comes from the original developers' public
profiles, as recorded in [artwork sources](icons.md#registered-sources). Images
are fetched to the user's cache, not bundled or relicensed as project assets. A
screenshot or future package must preserve applicable rights and cannot imply
endorsement.

## Locked source inputs

`Cargo.lock` records exact registry versions/checksums and Git commits. Some
upstream Git manifests use branch/tag references; builds use `--locked` rather
than silently resolving new heads. `deny.toml` admits crates.io and an explicit
Git-source set. The libcosmic revision is
`430f3d48a97d8779f9c1985473912e594ccca1ae`.

The native graph includes System76's libcosmic/panel/configuration code, Iced
and clipboard/windowing forks, the Winit fork, cryoglyph, and public Rust
utilities. Cargo metadata and `cargo tree --locked --features applet` identify
actual consumers; the lockfile, not this explanatory list, is the resolution
authority.

`eventsource-stream` parses the native router's read-only SSE progress stream;
`futures-util` supplies stream combinators for the existing reqwest transport.
They do not download model artifacts, proxy inference or own runtime processes.

The optional package `libcosmic` is explicitly imported under its Rust crate
name `cosmic`. `cargo xtask audit` requests the complete workspace and all
features, including the actual GUI dependency edges. An audit that unexpectedly
omits these Git sources is incomplete, even if its license stage reports
success.

The external runtime is llama.cpp b11856, commit `3642238cc`, under MIT. Hosting
uses
[Process Compose v1.122.0](https://github.com/F1bonacc1/process-compose/tree/v1.122.0)
under Apache-2.0. Its reviewed Linux amd64 release archive has SHA-256
`9b6dbc38324c0b0481f1cd1dd828ffdc78117129ec797678f4bf8c4023311281`. These are
separate executables, not bundled inference libraries. Preserve their
licenses/notices when packaging them; the Cargo audit does not audit external
binaries. Downloads of these tools require explicit installation, not an
implicit global package operation.

The crates.io `i18n-config 0.4.8` dependency is replaced with its upstream
implementation at `2ce7b1dfe9a8cc224288bf50cbc2a67a6de863c4`, where
[cargo-i18n PR #176][i18n-update] replaces archived `basic-toml` with maintained
`toml`. This is a pinned upstream patch, not a local replacement parser or an
advisory ignore. Its consumer remains the existing Fluent macro/configuration
dependency graph. Remove the patch when a compatible registry release includes
that change. The source retains the upstream MIT workspace license.

## License review boundary

The pinned graph has incomplete upstream Cargo license declarations. Several
libcosmic/configuration crates carry repository-level licenses rather than
metadata that cargo-deny can resolve automatically. The libcosmic and
DBus-settings source trees contain MPL-2.0 license text; clipboard/Iced sources
include MIT notices. `apply 0.3.0` has an Unlicense text in `License.md` without
a manifest expression.

### Verified metadata clarifications

`deny.toml` supplies exact-version clarifications for the following packages.
Each references the actual repository-level license relative to that package's
manifest and binds the text with cargo-deny's normalized XXH32 hash. These
clarifications identify source-backed licenses; they do not waive a
notice/source obligation or ignore an advisory. GPL admission below is a
separate, package-scoped policy decision.

| Package and version | Expression | Package-relative text | XXH32 |
| --- | --- | --- | --- |
| cosmic-config 1.0.0 | MPL-2.0 | `../LICENSE` | `3b6981e6` |
| cosmic-config-derive 1.0.0 | MPL-2.0 | `../LICENSE` | `3b6981e6` |
| cosmic-theme 1.0.0 | MPL-2.0 | `../LICENSE` | `3b6981e6` |
| cosmic-settings-daemon 0.1.0 | MPL-2.0 | `../LICENSE.md` | `0d4a6b51` |
| dnd 0.1.0 | MIT | `../LICENSE` | `fd9601dd` |
| mime 0.1.0 | MIT | `../LICENSE` | `fd9601dd` |
| iced_accessibility 0.1.0 | MIT | `../LICENSE` | `a244f17c` |
| xdg-shell-wrapper-config 1.11.0 | MPL-2.0 | `src/lib.rs` | `3d8a194e` |
| cosmic-panel-config 1.11.0 | GPL-3.0-only | `../LICENSE.md` | `03092a70` |

Source evidence at the locked commits:

- The configuration/theme packages sit under the
  [libcosmic MPL-2.0 license][cosmic-license]. Their own package trees contain
  no overriding license files or conflicting SPDX declarations. The
  [Iced subtree MIT license][iced-license] is the nearer governing text for
  `iced/accessibility`, not the outer libcosmic license.
- [DBus settings bindings][dbus-license] carries an MPL-2.0 root license at
  `eed01dd3609e90e3c8cd043656734c500956c793`; the generated proxy package sits
  directly below that root without a separate package license.
- [window_clipboard][clipboard-license] carries the complete MIT permission,
  notice and warranty text for its `dnd` and `mime` subpackages at
  `f68595ee0e62fbd6589f4709b5aaa5c3c7ea5f6c`. Neither package declares a
  different license. The `mime 0.1.0` clarification does not apply to crates.io
  `mime 0.3`.
- [xdg-shell-wrapper-config's sole Rust source][xdg-license] explicitly starts
  with `SPDX-License-Identifier: MPL-2.0`. This file-level grant takes
  precedence over inferring a license from the surrounding repository. The
  clarification hashes the complete marked source; it does not relicense
  unmarked panel files.

Raw-file SHA-256 values, independently identifying the reviewed texts:

| Text | SHA-256 |
| --- | --- |
| libcosmic LICENSE | `1f256ecad192880510e84ad60474eab7589218784b9a50bc7ceee34c2b91f1d5` |
| Iced LICENSE | `fc9086ba4eba4b77e4c603906362742bc3715ffa2c7810c6074fe7780d3a0398` |
| DBus bindings LICENSE.md | `e368885fd6ab4d78863ed9a0daac8a16b40a55c58395a9061f7c36b52a30536d` |
| window_clipboard LICENSE | `7faec985f3e9b1c4a1df646bfedc304ba67e409447f7ab7f4f591f68708d85dc` |
| xdg-shell-wrapper-config src/lib.rs | `58b201b0144e161a12b3110fe79d8ce969f883e62e33b8db210acaa730d2f2fa` |
| cosmic-panel LICENSE.md | `6ea843d9f221a2a7cb757d65c86368b9f207584ed0f2653e4b215a7702a7e1c7` |

Review the text, package scope and Cargo.lock again when upgrading upstream.
A changed hash invalidates a clarification and leaves ordinary license checking
in force. This is provenance-backed metadata, not independent legal clearance.

### License admission and distribution

Unlicense is accepted as a standard open-source license. `apply 0.3.0` carries
that text in `License.md`; missing manifest metadata is not a reason to reject
the license itself.

`cosmic-panel-config 1.11.0` has no package license expression or Rust SPDX
headers. The conservative interpretation is the
[repository GPL v3][panel-license] at locked commit
`ca20ce3185b3b4404f429d59d429c40b58a2ddef`. Its package tree has no overriding
license grant. The exact-version clarification uses `GPL-3.0-only`: it does not
infer permission for later GPL versions or borrow MPL from neighboring files. A
package-specific exception admits these terms; unknown licenses and unrelated
GPL packages are not globally admitted.

Project-authored files remain MIT. Distribution of a linked combined work must
respect GPL v3, including applicable corresponding-source, license and notice
requirements; it must not be advertised as an MIT-only binary. MPL-covered
dependencies retain their source/notice obligations too. Before distributing
binaries, review compatibility across the complete resolved graph and provide
the required license texts, notices, corresponding source and build inputs.
The Arch recipe aggregates notices using `cargo-bundle-licenses`, supplements
the reviewed missing-metadata texts, and installs this policy beside them.
Review aggregation warnings and license selection before distributing packages;
the tool does not certify completeness or compatibility. Corresponding-source
delivery is not automated. Audit success is dependency admission, not legal
clearance or a complete distribution-compliance bundle.
Recheck scope when updating the panel dependency or receiving upstream license
clarification. No publication is authorized by this policy.

## Advisory admission

The locked upstream graph contains unmaintained-package advisories:

| Package | Advisory |
| --- | --- |
| rustybuzz 0.20.1 | [RUSTSEC-2026-0206](https://rustsec.org/advisories/RUSTSEC-2026-0206) |
| ttf-parser 0.25.1 | [RUSTSEC-2026-0192](https://rustsec.org/advisories/RUSTSEC-2026-0192) |
| zbus_names 4.3.5 | [RUSTSEC-2026-0371](https://rustsec.org/advisories/RUSTSEC-2026-0371) |
| zvariant 5.15.1 | [RUSTSEC-2026-0370](https://rustsec.org/advisories/RUSTSEC-2026-0370) |
| zvariant_derive 5.15.1 | [RUSTSEC-2026-0372](https://rustsec.org/advisories/RUSTSEC-2026-0372) |

These are maintenance findings, not a claim of five exploitable vulnerabilities.
They enter through the pinned native stack. The owner-approved
[risk-based policy](decisions/0004-dependency-risk-policy.md) accepts these five
IDs only. Reviewed on **2026-10-25**, with maintainer review due by
**2027-01-25**, or earlier on a dependency/advisory/exposure change. These
exceptions do not cover new vulnerability IDs. cargo-deny 0.20.2 supports
reasons but not expiry or version constraints for advisory ignores: the date is
a human review obligation, not an automatically enforced expiry. Unused ignores
fail the audit so obsolete entries must be removed.

### Accepted exposure and removal conditions

- `rustybuzz 0.20.1` is retained by `usvg 0.45.1` through both Iced SVG
  renderers. Replacing only Herd's own text/rendering dependencies does not
  remove that edge. Residual risk is unsupported shaping of SVG text in native
  theme/toolkit inputs. Herd's remote-artwork loader accepts raster formats, not
  SVG; installed theme SVGs still use the native renderer. Bounded artwork
  decoding is not a sandbox or proof of parser safety. Accept pending upstream
  renderer migration to `harfrust`;
  remove the ignore when the dependency leaves the graph.
- `ttf-parser 0.25.1` is retained through `fontdb` (COSMIC text and SVG) and
  `owned_ttf_parser`/`ab_glyph` (Wayland window decorations). Removing a single
  consumer does not eliminate the package. The maintained replacements named by
  the upstream ecosystem need integration in those consumers, not an alias that
  labels unrelated APIs as `ttf-parser`. Residual risk is unsupported parsing of
  installed fonts and native theme/toolkit SVG font inputs, not SVG accepted by
  Herd's remote-artwork loader. Accept the existing toolkit dependency;
  remove the ignore when all consumers migrate, for example to `skrifa`.
- `zbus_names`, `zvariant` and `zvariant_derive` are merged into zbus 6, but the
  pinned native stack still requires zbus 5. Upgrading Herd's direct dependency
  alone leaves that stack and its advisories in the graph. Native toolkit,
  portal, accessibility and generated proxy consumers must migrate together.
  `zbus_names` validates D-Bus names; `zvariant` parses wire values from local
  peers; `zvariant_derive` generates serializers at build time. Local peers are
  not inherently trusted, and parser defects remain a risk. These advisories
  report consolidation, not a specific flaw. Accept the native zbus 5 graph;
  remove each ignore when its standalone package leaves the graph after a
  compatible zbus 6 migration.

Do not maintain API-shim crates, relabel archived code as a new version, disable
native capabilities merely to hide these edges, or turn off unmaintained checks.
Maintenance status alone does not require Herd to fork or migrate the entire
native toolkit. Reconsider acceptance immediately if an advisory gains concrete
vulnerability information; do not assume an ID-specific ignore distinguishes
changed advisory content. New vulnerability findings remain blockers. Upstream
patches must be pinned, reviewed and tested before use.

## Publication scope

No binary upload, registry publication, signing or vendored dependency archive
is configured. The prepared Arch recipe generates a license-notice bundle but
does not publish it or provide corresponding-source delivery. The AppStream
project-license field identifies the combined application's GPL v3 licensing;
project-authored source and metadata remain MIT. It is not a
substitute for reviewing the resolved combined work. CI source configuration is
not proof of hosted acceptance or permission to waive upstream findings.

[cosmic-license]: https://github.com/pop-os/libcosmic/blob/430f3d48a97d8779f9c1985473912e594ccca1ae/LICENSE
[iced-license]: https://github.com/pop-os/libcosmic/blob/430f3d48a97d8779f9c1985473912e594ccca1ae/iced/LICENSE
[dbus-license]: https://github.com/pop-os/dbus-settings-bindings/blob/eed01dd3609e90e3c8cd043656734c500956c793/LICENSE.md
[clipboard-license]: https://github.com/pop-os/window_clipboard/blob/f68595ee0e62fbd6589f4709b5aaa5c3c7ea5f6c/LICENSE
[xdg-license]: https://github.com/pop-os/cosmic-panel/blob/ca20ce3185b3b4404f429d59d429c40b58a2ddef/xdg-shell-wrapper-config/src/lib.rs
[i18n-update]: https://github.com/kellpossible/cargo-i18n/pull/176
[panel-license]: https://github.com/pop-os/cosmic-panel/blob/ca20ce3185b3b4404f429d59d429c40b58a2ddef/LICENSE.md
