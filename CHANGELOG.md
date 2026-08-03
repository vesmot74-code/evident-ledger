# Changelog

## Versioning note (2026-08)

This repository currently has three parallel release-candidate lines,
covering different scopes of the same `stage-13.7-release-candidate` /
`stage-14` work:

- `v1.2.0-rc1` — contract milestone: TSA/RFC3161 verification hardening,
  fail-closed startup guards, warnings audit. This is the version referenced
  in the project's public write-up.
- `v0.13.7-rc2` — adds TSA proof-boundary formalization and identity/billing
  governance findings on top of the above.
- `v0.13.8-rc1` — adds signing-key operational governance (ADR +
  readiness audit) on top of `v0.13.7-rc2`.

`0.13.x` is the working line converging toward the next `1.x`. All three
tags remain valid historical pre-releases; `v0.13.8-rc1` is the most
complete snapshot as of this note.

## [Unreleased]

### Changed

- Startup failure handling: config / database / signing / network / runtime paths use categorical typed exit codes instead of panic (Stage 13.7 RC stabilization; commit `68747cf`).

### Fixed

- Live-server integration test fixtures: API key lookup hash drift aligned with `hash_api_key_for_lookup`; `dev_tariff_switcher` no longer uses a hardcoded chain id (commit `7de624e`).

### Notes

- No production API behavior changes in this RC stabilization scope.
- See `docs/audits/STAGE_13_7_RC_TEST_STABILIZATION.md`. Stage 13.7.2-A/B security and deferred doc findings remain tracked separately.

## [1.0.0] - 2026-07-08

### Added

- Initial public release.
- Cryptographic evidence engine core.
- SHA-256 evidence hashing.
- Immutable evidence chain model.
- RFC 3161 TSA integration.
- Deterministic PDF evidence reports.
- Cyrillic-compatible PDF rendering with embedded DejaVu fonts.
- Multi-tier trust model:
  - Personal Proof.
  - Legal Compliance.
  - Immutable Audit.
  - Enterprise Identity.

### Documentation

- Added public README.
- Added security model documentation.
- Added protocol specification.
- Added contribution guidelines.
