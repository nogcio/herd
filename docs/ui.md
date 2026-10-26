# COSMIC interface conventions

## Composition and ownership

The management application uses libcosmic's application navigation model for
Models, Find models and Downloads. Models is a searchable, name-sorted library;
Downloads contains router-reported active and incomplete transfers, not a second
download history or cache registry. Completed entries appear in Models.
Local GGUF import is a header action, not a permanent sidebar page.
Model-specific settings use the native context
drawer, with independently expandable sections and a save footer. Confirm
discard/reload through the application dialog overlay. The panel applet stays a
compact companion and activates the main application instead of hosting editors.

Model-list refresh is an accessible icon action in the native window header,
not an extra page toolbar. The Add model menu offers Find models and Import
GGUF. Theme surfaces, typography, selection, spacing, menus and window chrome
come from libcosmic, not a fixed application palette or density preference.
Model rows share one native list surface. Selection identifies the open settings
draft, not loaded state; loaded state is explicitly labelled In memory.

Use native search input with Enter/clear, full-width repository and variant
list rows, a Back action, and one scroll container. Do not stack search results,
repository files and an acquisition form on the same page or nest fixed-height
scroll views. Variant selection opens the native download confirmation dialog;
Cancel and Escape submit no operation. Preserve results when returning from a
repository and reject stale asynchronous completions.

Model overflow menus contain settings and **Remove…**. Removal uses a native
dialog with a neutral Cancel and an explicit destructive action naming its
scope: unregister the preset while keeping the GGUF, or delete native cached
files. Do not put a destructive one-click trash button next to Start. Running
models and transfers must be stopped/paused separately; unsaved target drafts
must be saved or discarded first. Closing a dialog never cancels accepted
backend work.

Use `widget::settings::section` and settings items with parameter descriptions,
native controls and theme spacing. Do not replace the native header/navigation/
drawer shells with hand-built imitation chrome. Keep explanations next to the
parameters they describe; routine backups, inheritance implementation and
service-lifetime details belong in documentation, not repeated help paragraphs.
Errors, required consent and dirty-draft confirmations remain visible.

The maintained patterns were reviewed in the exact
[libcosmic settings widgets][widgets], [context drawer][drawer], and
[COSMIC Settings application][settings]. Use their public APIs without copying
the Settings page/plugin framework into this standalone application. Their
source licenses and the project's existing dependency obligations remain
separate.

## Panel and popup

Both panel orientations use one native symbolic icon button. Discovery/action
errors use a warning icon and active downloads use a download icon. Use
COSMIC's panel sizing, padding and hover/pressed state, without separate badge
backgrounds or activity animations. Model initials use a monospaced font.

The popup contains a product header with icon-only refresh, operation/error
feedback, and scrollable model rows. Artwork
falls back to initials; ready state uses both accent and text. Keep routine tips
and setup tutorials out of the popup, retaining errors and required
confirmations.

Compact popup transfer actions sit below the model description so a long Resume
label cannot squeeze the model identity into a narrow column. Unknown,
unavailable and stopping instances have no misleading Unload action.

Load uses a neutral native action. Unload stays visually quiet until
selected, then offers inline destructive confirmation in the popup or a native
dialog in the management window. Other
model actions are disabled during confirmation. Background refresh does not
disable actions: the backend revalidates admission when they are requested.

The settings drawer opens Context and GPU sections initially. Its footer offers
Discard and Save; Discard restores that draft's last-read explicit values
without rereading configuration, changing another draft, or writing anything.
Reload in the drawer header reads a fresh backend snapshot and confirms dirty
replacement. All supported native parameters remain available in expandable
groups.

Resume inspection has its own visible progress/error state. Leaving its page,
Cancel or Escape invalidates that review without clearing metadata search drafts
or canceling an accepted transfer. Late completions cannot reopen a
confirmation. Escape on an editor discard/reload dialog keeps the draft and
cancels pending window close. Setup inputs retain visible labels when populated;
search errors are distinguished from repository inspection and acquisition
failures.

Application exit and explicit window-manager close requests use the same
dirty-draft guard. The application supplies `on_app_exit`, subscribes to native
close requests and sets `exit_on_close(false)` so the window manager cannot
bypass that guard. Removal-plan reviews carry a generation token; cancellation
invalidates late results without canceling already accepted backend work.

## Literal values and rendering

Keep all parameter sections in one scrolling panel. Build fields only for
expanded sections; multiple sections can stay open together. Use responsive
native settings rows: text/enum controls expand below their label at narrow
widths instead of being squeezed into a fixed-width input column. Preserve
native values that do not fit a finite control rather than rounding or guessing
them. Known boolean and enum values use togglers/dropdowns; arbitrary GPU layer
counts, paths and unrecognized literal forms retain text input. The full
explicit option set is still saved atomically by the backend; visiting a
category never writes.

Assign stable text-input identities for a draft's lifetime. Native lazy widgets
cache each row using its key/value, override state, enabled state, input
identity and theme spacing. Unrelated field edits are not cache
dependencies. Backend notifications refresh the existing client instead of
reconnecting for every snapshot signal. These are structural reductions in work,
not measured frame-rate guarantees. Native acceptance must still test actual
input and frames; debug-build timings must not be presented as release-build
performance.

Enable libcosmic's explicit `wgpu` feature for the GUI. With default features
disabled, Wayland support alone compiles the wgpu library but does not select
the GPU renderer: iced otherwise uses tiny-skia on the CPU. Keep the maintained
tiny-skia fallback for machines without a usable GPU backend. Never force a
particular graphics driver or change system GPU/security settings.

The expanded-drawer regression test performs real headless layout at multiple
widths and clipped CPU rasterization, not just element construction. It also
requires the production renderer type to retain a primary GPU backend and a
CPU fallback. Timing output is diagnostic only; tests assert bounds and valid
pixel output, not machine-dependent performance thresholds. Headless timings do
not establish live compositor frame times, selected adapter or native scrolling.

CPU-rendered fixture images can support layout review but are not screenshots
of a running panel or proof of interactive acceptance. Gather is the adopted
application icon, with source assets and package integration; see
[icon provenance](icons.md). Source/package integration does not install assets
globally or establish live desktop acceptance.

## Localization

English and Russian Fluent resources are embedded, including in debug builds.
The desktop locale is negotiated at startup, with English fallback. Labels,
parameter explanations, controls, statuses, consent and accessible action names
are localized. Model names, native keys, enum values, paths and CLI JSON remain
data, never translated. Native values use runtime syntax such as `0.7`, `auto`
and `q4_0`, regardless of GUI language. Technical backend/runtime diagnostics
remain English; the UI does not guess translations by matching error strings.

[widgets]: https://github.com/pop-os/libcosmic/tree/430f3d48/src/widget/settings
[drawer]: https://github.com/pop-os/libcosmic/blob/430f3d48/src/app/context_drawer.rs
[settings]: https://github.com/pop-os/cosmic-settings/blob/f1d00fc06e18203e880771617657067c10c43321/cosmic-settings/src/app.rs
