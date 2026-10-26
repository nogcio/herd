# ADR 0004: Risk-based dependency admission

Status: accepted.

Use the upstream native stack rather than maintaining replacement crates or
forking COSMIC solely to eliminate maintenance warnings. Dependency admission
distinguishes reported vulnerabilities, maintenance status and license terms.

- Known vulnerability findings remain blockers; maintenance acceptance does not
  waive them. Model/data safety, source restrictions and secret checks remain
  unchanged.
- Maintenance-only advisories may be accepted by exact ID with owner approval,
  dependency paths, exposure, rationale, removal conditions and a review date.
  Reassess on dependency or advisory changes. Do not disable the category.
- Accept legitimate open-source licenses with their obligations. Missing Cargo
  metadata can be clarified from exact source evidence, never invented. Copyleft
  admission is not permission to distribute an MIT-only combined binary.
- Keep review lightweight: use `deny.toml` and
  [third-party obligations](../third-party.md), not a new exception framework.
  Review dates are maintainer obligations; the pinned auditor does not enforce
  expiry. Human review remains required.

The current five maintenance acceptances avoid a coordinated toolkit migration
without claiming the affected parsers are risk-free. Unlicense admission and
the exact panel package's GPL v3 admission replace arbitrary license rejection
with explicit compliance obligations. Publication approval and preparation of
required source/notices remain separate from a passing source audit.
