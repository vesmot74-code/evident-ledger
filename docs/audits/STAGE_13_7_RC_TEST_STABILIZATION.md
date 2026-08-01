# Stage 13.7 RC Test Stabilization

Branch: `stage-13.7-release-candidate`

Date: 2026-08-02

## Scope

This document records release-candidate stabilization work completed after TSA trust hardening, limited to startup failure handling and live-server integration test fixtures.

- Startup exit code refactor completed in commit `68747cf`
  (`startup: replace panic! with typed exit codes (config/db/signing/network/runtime)`)
- Live-server integration test fixes completed in commit `7de624e`
  (`test: fix live-server integration test fixtures (Stage 8.1 hash drift + hardcoded chain_id)`)

This document does **not** redefine or close Stage 13.7.2-A / Stage 13.7.2-B security or documentation findings.

## Completed Items

✅ **Startup failure handling**

- config / db / signing / network / runtime failures use typed exit paths
- categorical server exit codes documented in `docs/design/STARTUP_EXIT_CODES.md`
- TSA trust validation `exit(1)` behavior from Stage 13.7 is unchanged

✅ **Integration test stabilization**

- Fixed API key hash drift:
  - tests previously used a direct SHA-256 of the full API key string
  - production lookup uses `auth::api_key::hash_api_key_for_lookup`
  - corrected live-server suites:
    - `v1_events_idempotency`
    - `v1_events_validation`
    - `v1_proof`
    - `v1_public_proof_wire`
    - `v1_verify`
    - `v1_verify_chain`
    - `v1_verify_file`

✅ **`dev_tariff_switcher`**

- Removed hardcoded `chain_id`
- Test creates an independent `chain_id` on each run

## Validation Evidence

Command:

```bash
cargo test --test v1_events_idempotency \
  --test v1_events_validation \
  --test v1_proof \
  --test v1_public_proof_wire \
  --test v1_verify \
  --test v1_verify_chain \
  --test v1_verify_file \
  --test dev_tariff_switcher
```

Results:

| Suite | Result |
|-------|--------|
| `dev_tariff_switcher` | 2/2 |
| `v1_events_idempotency` | 1/1 |
| `v1_events_validation` | 5/5 |
| `v1_proof` | 12/12 |
| `v1_public_proof_wire` | 2/2 |
| `v1_verify` | 5/5 |
| `v1_verify_chain` | 4/4 |
| `v1_verify_file` | 8/8 |

## Non-Blocking Known Items

The following documents contain separate Stage 13.7 follow-up items and are **not** in scope of this document:

- `docs/audits/STAGE_13_7_DOC_AUDIT.md`
- `docs/audits/STAGE_13_7_SECURITY_DECISION.md`

Existing Stage 13.7.2-A/B security work and deferred documentation findings remain tracked separately. This document does not close or modify those items.

## Release Candidate Status

Stage 13.7 RC stabilization scope is complete.
