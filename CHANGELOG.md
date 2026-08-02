# Changelog

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
