# Security policy

Herd is pre-release. No production-release schedule or
response time guarantee has been established. Fixes target the development
branch.

## Reporting

Do not put exploit details, credentials, prompts, or private unit/environment
data in public issues. A private reporting channel must be verified for the
eventual GitHub repository; it is not established by this source tree. Ask
maintainers for a private contact with a minimal non-sensitive message before
sharing details. Do not assume that an issue or platform report is confidential.

Include the affected revision, impact, prerequisites, and a minimized/redacted
reproduction. Never test against someone else's desktop or model endpoint.

## Trust boundaries

- The application runs with the user's privileges. Explicit host commands run
  configured llama.cpp and Process Compose executables; these must be trusted.
  It is not a process/container/filesystem sandbox or a cgroup memory limiter.
- Native preset validation restricts supported settings and disables startup
  loading; it does not certify the safety or memory requirements of model files.
  Parser errors must not expose configuration snippets or credentials.
- Application operation locking does not prevent external clients from using a
  GPU or accessing the router directly. Readiness does not establish endpoint
  ownership, inference quality, or safe idleness.
- Artwork is untrusted network/file input. Host/redirect/TLS/size/decoder limits
  are part of the security contract. Failures are decorative, not service
  authorization.
- No system-manager access, privileged helper, implicit autostart or implicit
  model download is supplied. Public Hub inspection is read-only; explicit
  acquisition requires reviewed metadata and confirmation. Native transfers
  follow current main, not an immutable fetch pin or hard byte quota; reviewed
  file digests are checked before acquired models can load. Explicit host
  startup opens a loopback router listener; Process Compose management uses a
  private Unix socket. Loopback and CORS do not constitute authentication
  against other local programs or a full browser sandbox. Do not expose either
  management API to a LAN or untrusted clients.
- Diagnostic commands do not send inference requests. The GUI automatically
  loads optional configured artwork from approved public image hosts and writes
  its local cache; there is no separate artwork CLI command.

Dependency audits, secret scanning, and code review are complementary checks,
not a guarantee that an application or optional image is free of
vulnerabilities. Expanding runtime/network authority requires a reviewed
contract and failure tests.
