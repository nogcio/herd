# ADR 0008: Concrete engine integrations and cross-engine admission

Status: implemented in source; native runtime, cgroup and desktop acceptance
remain separate and require owner authorization.

## Decision

Keep one Rust package and one typed management service. Concrete engine modules
own native configuration, transport, lifecycle and acquisition. There is no
dynamic plugin framework, arbitrary command template, inference proxy, generic
process supervisor or second model registry. An OpenAI-compatible inference API
does not establish a management contract.

`engines::llama_cpp` owns INI presets, router HTTP, native HF cache and GGUF
acquisition. `engines::colibri` owns the selected native setup, concrete
resource editor and native multi-file acquisition. Common `controller` admission
checks competitors; common `hosting` implements the limited reviewed Process
Compose mechanism both integrations consume. No universal options map translates
between engines. The UI uses each engine's actual options and capabilities.

The management API is `Backend2`, version 2. Runtime-qualified targets carry the
runtime ID, native profile ID and a reviewed configuration revision. Model names
alone cannot select a different engine. `Report` includes independent runtime
and transfer observations; its existing model library remains llama.cpp-native.
The window offers an engine selector with Models, Find models, Downloads and a
native settings drawer. Colibri load can select verified weights and launch in
one explicit operation; downloading and saving settings never load anything.

## Native authority and identities

llama.cpp keeps native INI authority. Colibri keeps one native `setup.json` at
`COLI_SETUP_HOME`, or the native XDG data/home fallback. The backend, not the
calling CLI, supplies the environment and installed executable lookup. Native
model directories, catalogue and download manifests supply artifact facts.
Integrity receipts are operational records, not editable model profiles.

An ID combines the engine kind and SHA-256 of the lexical absolute native
configuration path. It does not prove process ownership. Different paths,
including symlink aliases, have different IDs. One native configuration location
per engine is connected currently; this is not automatic process/port discovery
or arbitrary multi-installation support.

Profiles, artifacts, owned instances and transfers remain different identities:

- Colibri's `configured` profile refers to its single selected native setup.
- An acquisition plan binds repository, immutable commit, destination, file set
  and advertised hashes; present byte counts do not establish integrity.
- An owned host journal freezes source, revision, endpoint and an
  instance-specific process/model identity. Native edits cannot redirect its
  stop operation.
- Transfers have their own host, immutable plan, progress and terminal result.
  They never borrow a model instance's lifecycle state.

## Admission and ownership

One service-owned mutation is admitted at a time. A submitted download operation
finishes after its independent transfer host accepts work, not when bytes
finish. Active or uncertain transfers still block model starts. Each new
load/acquisition checks both connected engines. Native source changes and
competitors are checked again after long metadata/digest verification. Unknown
states fail closed; the application never unloads a competitor automatically.
This is not a machine-wide GPU reservation or control over external clients
bypassing Herd.

Colibri starts with an installed Python launcher and installed family engine,
using only a validated `serve` invocation. Native args/environment are parsed as
bounded data, not executed as command strings. Unsupported options fail without
silently dropping settings. Ambient execution overrides are not inherited.
Loopback binding, private temporary state and disabled implicit OpenMP tuning
prevent accidental exposure, foreign PID-file replacement and unrequested
benchmarking. `setup`, `start`, `stop`, build, conversion and tune flows are not
Herd's management implementation.

Process Compose hosts inference and acquisition in separate independent systemd
user scopes with private Unix sockets, literal entrypoints, no dotenv, no watch
restart and no automatic restart. A stop first matches exactly the journal's
single process through the bounded native UDS API, then stops that owned
project. There is no PID-file, command-line, environment or process-name search.
External listeners are observed but cannot be adopted or stopped. An
unidentifiable lost host blocks a replacement; no automatic cleanup guesses that
inference exited.

Colibri launch completion means the host accepted launch. Later observation
matches `/v1/models` against the recorded model identity. This establishes a
gateway observation, not inference quality, idle requests or guaranteed future
engine health. Timeout or lost acknowledgement never sends compensating unload.

## Acquisition and editing

The installed Colibri catalogue and downloader APIs are consumed through an
embedded, bounded Python caller. The caller never enters setup/build/conversion,
never reads Hub credentials and never implements its own transfer/retry/cache
algorithm. Colibri owns those operations. File URLs use an immutable inspected
commit, HTTPS and a restricted Hub/CDN host policy without proxies.

Existing complete files are checked with native digest verification before
reuse; the native downloader's size-only early return is not accepted as
integrity. Existing corrupt final files are not overwritten. Native partial
files remain resumable after explicit pause/review. Completion never changes the
selected profile. See the current supported scope and limitations in
[Colibri](../colibri.md).

Native Colibri settings save as a complete explicit draft with bounded concrete
CLI/environment keys. Authentication and unrelated JSON values are preserved,
but credentials never enter the editor protocol. Saves retain exact private
backups, detect external revision changes and atomically replace native data.
Changing the selected artifact follows the same rules. Edits leave a running
instance unchanged and pending; neither watcher nor save restarts it. External
non-cooperating writers can still race the final check/rename: no filesystem
compare-and-swap or namespace isolation is claimed.

## Reference and acceptance

Colibri reference: [e0db083][colibri], including `c/coli`, `c/setup_flow.py`,
`c/setup_catalog.py`, `c/setup_download.py`, `c/family_registry.py` and
`c/openai_server.py`. Process Compose v1.122.0 supplies entrypoint, daemon,
shutdown and `/processes`/`/project/stop` semantics. These are reviewed sources,
not installed-version pins or certification of arbitrary other versions.
Unsupported response/API shapes fail; there is no fallback to a less safe flow.

Tests use bounded subprocesses, temporary native files, HTTP/UDS mocks and a
native API test double. They check scoped ownership, stale configurations,
unchanged competitors, native transfer reuse/corruption and editor behavior.
They do not establish real model inference, live transfers, systemd cgroup
survival or COSMIC input/frames. No live model is a test fixture without
explicit owner authorization. Human review remains required.

An installed `Backend1` cannot be replaced through new clients' `Backend2`
methods. Deployment requires an explicitly authorized management-service
replacement after accepted operations finish; source checks do not install or
restart it. Existing inference must remain untouched.

[colibri]: https://github.com/JustVugg/colibri/tree/e0db083cdc9e563e0766cc6fc661ef23c32298e0
