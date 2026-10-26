# Native configuration

## Files and authority

`server.json` contains hosting paths, library directories, router endpoint,
working directory, and private state directory. It is not a model registry. See
[the hosting template](../templates/server.json). All paths are absolute; the
endpoint must be an explicit IPv4 loopback HTTP origin and port. Redirects and
proxy use are disabled for management transport.

`models.ini` is a native llama.cpp preset file consumed by the router itself:

```ini
version = 1

[*]
load-on-startup = false
stop-timeout = 30

[example-model]
; @name=Example model
; @short-name=EX
model = /absolute/path/example.gguf
ctx-size = 8192
n-gpu-layers = auto
```

Section IDs are model IDs. `[*]` supplies native defaults. The application does
not write a second model database or inspect launcher/process arguments to infer
settings. File registration never moves weights, enables autostart or downloads.

Automatic system/user llama.cpp `config.ini` files are not another supported
configuration source. Hosting fails closed if either exact host path exists or
cannot be inspected, without reading contents or relocating HOME/cache. The
controller rechecks recorded host paths before reload/load/download dispatch;
independent creation after a check remains unsupported, not atomically isolated.
See
[automatic native configuration](architecture.md#automatic-native-configuration).

## Validation

Reads are bounded to 1 MiB and 128 profiles. Duplicate sections/options and
unsupported keys are errors, not silently ignored profiles. Values are literal
single lines without shell evaluation, continuation or variable expansion.
Native model/projector paths are absolute. Startup loading must be false.

Literal values cannot contain native comment markers `#` or `;`, including in
paths. Section names cannot have boundary whitespace, and headers/options cannot
be indented; ambiguous native/parser interpretations fail closed. Spaces and
tabs around ordinary assignment separators remain supported.

Reads open files with nonblocking flags and validate the opened descriptor as a
regular file. This rejects FIFOs and other special files without waiting for a
writer, but does not impose a hard deadline on arbitrary regular filesystems.
Artifact symlinks resolving to regular files are supported. Preset symlinks may
be read, but all preset mutations reject them: select the authoritative regular
preset file explicitly rather than replacing its link.

The current supported native options are:

- `model`, `hf-repo`, `hf-file`, `alias`, `tags`.

Primary artifact selectors (`model`, `hf-repo`, `hf-file`) belong to individual
profiles, not shared defaults. A profile cannot combine `model` and `hf-repo`;
`hf-file` requires `hf-repo`. Local model profile IDs cannot contain `:` because
native cache tag identities can otherwise merge an unreviewed Hub selector.
Implicit cache tags accept only alphanumeric characters and underscores;
other tags require an explicit `hf-file` to avoid native regex ambiguity.

- `ctx-size`, `parallel`, `n-gpu-layers`, `gpu-layers`, `n-cpu-moe`,
  `moe-cache-mib`, `fit`, `flash-attn`.
- `cache-type-k`, `cache-type-v`, `batch-size`, `ubatch-size`, `threads`,
  `threads-batch`, `ctx-checkpoints`, `cache-ram`.
- `no-context-shift`, `context-shift`, `jinja`, `reasoning`, `reasoning-format`.
- `temp`, `top-p`, `top-k`, `min-p`, `stop-timeout`, `load-on-startup`,
  `dedup-cache-models`, `mmproj`, `no-mmproj`.

Unknown options are retained in a manually edited file but block admission until
the implementation explicitly supports them. Runtime tools, arbitrary commands,
network listeners and autoload flags are not accepted profile options. Native
argument compatibility remains the pinned runtime's responsibility; a valid INI
file is not proof that weights fit in memory or run successfully.

`hf-repo` does not grant download permission: normal inference children run
offline and explicit load requires existing local weights. Search and
acquisition are separate, explicitly authorized [operations](downloads.md),
never inferred from profile registration or a missing file.

## Presentation comments

Optional per-section comments are ignored by llama.cpp and read by the UI:

```ini
; @name=Example model
; @short-name=EX
; @icon=applications-science-symbolic
; @icon-url=https://huggingface.co/organization/repository/resolve/COMMIT/logo.png
```

Short names are up to four ASCII characters. Sources are explicit, never guessed
from labels. Invalid artwork falls back to initials and does not block runtime
actions. See [artwork restrictions](icons.md). No separate artwork registry
exists.

## Live edits

Parent-directory watches detect atomic editor saves, creation and replacement.
Events coalesce before a bounded reread. New sections appear without restarting
the application. No observation callback reloads runtime presets or stops
models.

Edits are pending until a safe explicit start: all runtime entries must first be
stopped, then the application requests the router's preset reload, checks the
file revision again, and loads only the requested model. There is no
auto-restart on save. Invalid/deleted files are errors and block starts.

The GUI/CLI save path uses an inspected SHA-256 revision, preserves unrelated
file lines and atomically replaces the file. Known concurrent edits are
rejected; external editors do not participate in the application's operation
lock, so the last check/rename is optimistic rather than a filesystem
compare-and-swap.

The complete GUI editor keeps explicit model options separate from inherited
`[*]` defaults. Reset removes an override; it does not write a guessed runtime
default or duplicate shared options. A whole-form save validates the resulting
file before atomically replacing it. Failed validation, stale revisions and
literal/injection errors leave all profiles unchanged. Existing comments and
unmodified lines, including unrelated line endings, remain intact. A no-op save
does not rewrite the file or create a backup.
