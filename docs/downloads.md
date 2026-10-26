# Model acquisition

Cache mutations validate the bounded native unfinished-download journal before
dispatch. Recorded partial paths must name hexadecimal blob
`.downloadInProgress` files
inside their repository's native cache directory; malformed or escaping paths
block mutation rather than being passed to native cleanup. Snapshot symlinks
must keep both their immediate unlink targets and resolved artifacts inside the
repository. Independent external cache edits can still race admission; this is
not a filesystem sandbox or an atomic transaction with llama.cpp.

## Management window

Open **Find models** in the sidebar or the **Add model** menu. Enter a search
term or literal `namespace/repository`, then press Enter or **Search**. Choose a
repository result and a variant showing its quantization, filename and size.
The native confirmation dialog requires an explicit **Download** action. Back
returns to the previous search results; Cancel never submits a transfer.

The initial interface is limited to unambiguous single-file GGUF models with an
advertised LFS SHA-256. Multiple matching files, split models, sidecars, remote
`preset.ini` files and private/gated repositories are not supported. Unsupported
choices are not silently guessed. Artwork is not inferred from quantizer names.

After acceptance, the **Downloads** view and popup show native download
progress. **Pause download** keeps partial files. **Resume download** opens a
new review; it is not an automatic retry. Native transfer/cache lifetime belongs
to llama.cpp, not the window or a CLI process. Closing UI or ending a status
wait does not pause or delete a transfer. No model loads after download
completion.

Progress comes from native `/models/sse` events, not `/models` list fields.
When the stream has not supplied counters or disconnects, the UI shows an
indeterminate bar rather than `0 / 0`. Backend stream reconnection never resumes
a paused download. Model file acquisition is not used to test the progress
parser: automated tests use bounded fake router responses.
Late events for IDs no longer actively downloading are ignored.

New acquisitions require existing inference and transfers to be stopped. This
conservative rule avoids native catalogue refresh side effects on running
models. A stale pause action refuses to unload a model that has begun inference.

## Revision, size and integrity semantics

Inspection records the repository commit, selected file, LFS size and SHA-256.
The application rechecks metadata immediately before submission and rejects a
changed plan or an advertised size above the authorized ceiling.

**The b11856 native download API follows current `main`, not an immutable commit
URL.** The displayed size and CLI ceiling are metadata admission, not an
enforced bandwidth/disk quota. The repository can change between inspection and
native transfer. Do not describe the plan as a pinned network fetch or hard byte
cap.

The private acquisition receipt records the reviewed artifact, not runtime
configuration or a duplicate cache catalogue. Before loading a model acquired
through this application, its local size, GGUF header and SHA-256 must match the
receipt. A mismatch blocks loading and does not delete or replace the file.
`verify` and `download --wait` verify completion without starting inference.
Externally managed cache files are not thereby certified by the application.

Already cached, matching bytes are reused without another transfer request.
Native acquisition requests an empty sidecar list. Normal inference children
inherit offline mode; explicit download workers use their native acquisition
mode without inference presets. No custom downloader or model registry is added.

## Native cache resolution

The pinned `/models` response supplies no local artifact path. Herd inspects the
native HF cache read-only, scoped to the requested repository's `refs/main` and
current `snapshots/<revision>`. Selection must identify an unambiguous single
GGUF or an explicit native `hf-file`; split models, sidecars and remote presets
remain unsupported. Acquired files must still match their reviewed receipt's
size, header and digest. Regular artifact symlinks are supported within the
validated repository boundary, not arbitrary escaping links.

Explicit router startup records the selected native cache root, both frozen
automatic-config paths, endpoint and server path in private `cache-root.json`
after readiness. This is operational identity, not a catalogue or another
registry. Cache operations and new model starts fail closed if identity is
missing or mismatched. Existing inference remains running; establishing identity
requires an explicitly authorized router replacement, not a backend restart.
No router stop/restart is automatic.

Before download-child dispatch, the controller rechecks the recorded host's
automatic-config paths, not the reconnecting backend's environment. Existing or
uninspectable automatic configuration blocks dispatch without reading its
contents. Creating configuration after the check remains an unsupported
independent-filesystem race; see
[hosting constraints](architecture.md#automatic-native-configuration).

## CLI

Use `cargo run --locked --bin herd --` instead of the
executable below when running from source:

```sh
herd --json search SmolLM2-135M
herd --json files bartowski/SmolLM2-135M-Instruct-GGUF
herd --json download-plan bartowski/SmolLM2-135M-Instruct-GGUF:Q4_K_M
```

Review the returned `revision`, `filename`, `size_bytes` and `sha256`, then pass
the inspected values explicitly:

```sh
herd --json download namespace/repository:Q4_K_M \
  --revision COMMIT_FROM_PLAN --max-bytes SIZE_FROM_PLAN --yes
herd --json status
herd --json pause namespace/repository:Q4_K_M
herd --json verify namespace/repository:Q4_K_M
```

Without `--wait`, success means **accepted**, not completed. `--wait` observes
completion for up to 24 hours, verifies the artifact, and emits one final JSON
result. Timeout/CLI exit does not cancel the native transfer. Status/progress
can be inspected independently; there are no hidden prompts or automatic
retries. Resumption uses the same reviewed `download` operation and native
partial files.

## Small manual test candidate

[SmolLM2-135M-Instruct GGUF](https://huggingface.co/bartowski/SmolLM2-135M-Instruct-GGUF),
derived from HuggingFaceTB's SmolLM2-135M-Instruct, offers a small `Q4_K_M`
file. Inspect its current metadata before accepting a transfer. It is suitable
for testing acquisition mechanics, not representative of larger model quality.
Model files are not bundled or downloaded by automated tests. Review the model
card's applicable license separately.

## Removing a model

Use **Remove…** in the model's overflow menu. The backend reviews the exact
scope before showing a confirmation:

- A configured model is removed from native presets, with concurrent-edit
  detection and a retained preset backup. Its original GGUF is never deleted.
  Idle stale preset observations are hidden until the next explicit router
  configuration apply; active or unknown observations remain visible.
- A native cache entry uses `DELETE /models?model=…`. llama.cpp owns deletion of
  completed/partial files and its shared-cache reference rules. Herd does not
  walk or delete the model-cache directory itself. Active inference and
  transfers must be explicitly stopped/paused first; deletion never sends those
  commands.

Known native-preset references block cache deletion. External/custom references
into the cache are not a complete reference graph: deleting their source cache
entry can make them unusable, so review those references first. Independent
runtime clients can also race admission; the native delete API has no atomic
expected-state precondition. No stronger cross-client guarantee is claimed.

CLI callers use `removal-plan MODEL`, then
`remove MODEL --revision TOKEN --yes`; cache deletion additionally requires
`--delete-files`. A changed scope or native configuration rejects the request.
There is no automatic unload, retry or redownload after removal.
