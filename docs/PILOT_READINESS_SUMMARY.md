# Pilot Readiness Summary

Stage 12.0 — operations package for a **controlled** first pilot. Not a product feature release.

Security / smoke lineage:

- [audits/SECURITY_AUDIT_STAGE_11_2.md](audits/SECURITY_AUDIT_STAGE_11_2.md) — Critical=0, High=0 (closed blockers)
- [audits/STAGE_11_3_SUBSCRIPTION_ENFORCEMENT.md](audits/STAGE_11_3_SUBSCRIPTION_ENFORCEMENT.md)
- [audits/STAGE_11_4_WEBHOOK_RELIABILITY.md](audits/STAGE_11_4_WEBHOOK_RELIABILITY.md)
- [audits/STAGE_11_5_MEDIUM_FINDINGS.md](audits/STAGE_11_5_MEDIUM_FINDINGS.md) — M3 closed; M4 accepted; M5 deferred
- [audits/STAGE_11_6_PILOT_SMOKE_TEST.md](audits/STAGE_11_6_PILOT_SMOKE_TEST.md) — READY WITH LIMITATIONS
- [audits/STAGE_12_0_FINDINGS.md](audits/STAGE_12_0_FINDINGS.md) — C1 signing-key backup **Closed**
- [audits/CRITICAL_SIGNATURE_PERSISTENCE_INVESTIGATION.md](audits/CRITICAL_SIGNATURE_PERSISTENCE_INVESTIGATION.md) — empty `events.signature` on CLI `/events` **Resolved**
- [audits/P1_LEGACY_EVENTS_IDENTITY_FIELDS.md](audits/P1_LEGACY_EVENTS_IDENTITY_FIELDS.md) — identity fields on legacy `/events` **Resolved** (`c77172e`)

| Area | Document | Status |
|---|---|---|
| Deployment | [PILOT_DEPLOYMENT_CHECKLIST.md](PILOT_DEPLOYMENT_CHECKLIST.md) | Ready |
| Signing key ops | [SIGNING_KEY_OPERATIONS.md](SIGNING_KEY_OPERATIONS.md) | Ready (backup + restore drill 2026-07-23) |
| Rollback | [ROLLBACK_PROCEDURE.md](ROLLBACK_PROCEDURE.md) | Ready |
| Onboarding | [PILOT_ONBOARDING_RUNBOOK.md](PILOT_ONBOARDING_RUNBOOK.md) | Ready |
| Monitoring | [MANUAL_MONITORING.md](MANUAL_MONITORING.md) | Ready (interim; no `/health`) |
| SLA draft | [PILOT_SLA_DRAFT.md](PILOT_SLA_DRAFT.md) | DRAFT — internal review required |
| Stage 12 findings | [audits/STAGE_12_0_FINDINGS.md](audits/STAGE_12_0_FINDINGS.md) | C1 Closed |
| Signature persist (CLI) | [audits/CRITICAL_SIGNATURE_PERSISTENCE_INVESTIGATION.md](audits/CRITICAL_SIGNATURE_PERSISTENCE_INVESTIGATION.md) | **Resolved** — legacy `/events` persists before commit |
| Identity on legacy `/events` | [audits/P1_LEGACY_EVENTS_IDENTITY_FIELDS.md](audits/P1_LEGACY_EVENTS_IDENTITY_FIELDS.md) | **Resolved** — reject; Identity via `/v1/events` only |
| Pilot UX onboarding | [CLI_INSTALLATION.md](CLI_INSTALLATION.md) | Stage 13.1 **Completed** |

Also referenced: [DEPLOYMENT.md](DEPLOYMENT.md), [DEPLOYMENT_FINDINGS.md](DEPLOYMENT_FINDINGS.md), [BILLING_MODEL.md](BILLING_MODEL.md), [`.env.example`](../.env.example).

---

## Known accepted limitations going into pilot

1. **Qualified TSA unavailable for paid plans** — event writes on `legal` / `vault` / `identity` may return `500 internal_error`; free-plan machine TSA commits work (Stage 11.6). **Do not implement TSA fallback in this stage** — operator responds per onboarding runbook.
2. **`paddle_price_id` must be ops-seeded** after fresh migrate before checkout works.
3. **Paid → paid plan change** is support-mediated, not self-service Dashboard.
4. **No CLI identity register** — Dashboard / HTTP only.
5. **CLI `server_identity.pub` pin** must match deployment public key for offline verify.
6. **`/health` deferred** (M5) — use [MANUAL_MONITORING.md](MANUAL_MONITORING.md).
7. **No migration `down` scripts** — rollback of schema requires DB backup discipline ([ROLLBACK_PROCEDURE.md](ROLLBACK_PROCEDURE.md)).
8. **Unmanaged CWD `./signing_key.bin` hazard** — may be a different key than `SIGNING_KEY_PATH`; never confuse the two ([SIGNING_KEY_OPERATIONS.md](SIGNING_KEY_OPERATIONS.md)).
9. **Paddle payment → webhook E2E** may need a reachable notification URL (Stage 11.6 partial).
10. **Signing-key off-host backup** — required; verified closed in Stage 12.0 C1 (maintain backups going forward).
11. ~~**CLI `/events` left `events.signature` empty**~~ — **Resolved** (see incident section below).
12. ~~**Legacy `/events` accepted identity fields without PoP**~~ — **Resolved** (see incident section below). Residual Low/Medium only: P2 dual idempotency, P3 weaker legacy validation ([CRITICAL perimeter](audits/CRITICAL_SIGNATURE_PERSISTENCE_INVESTIGATION.md)).

---

## Stage 13.1 — Pilot UX onboarding

Status: **Completed**

Implemented:

- Dashboard navigation links (home, Docs, Download CLI, Pricing, Account)
- Improved free plan presentation (`Free plan` / `No subscription` — no raw `none` / “нет”)
- First-run onboarding state when `server_commits == 0`
- CLI installation guide: [CLI_INSTALLATION.md](CLI_INSTALLATION.md) (binary name verified against GitHub release assets — pilot path uses `evident`, not `evident-gui`)

Deferred:

- Public download portal (`/download`)
- Apple notarization
- Automated installers

---

## Incident resolutions (pre-pilot)

### Legacy signature persistence incident

| | |
|---|---|
| **Status** | **Resolved** |
| **Fix** | `8194f6c` — persist via `persist_event_signature` before commit on legacy `POST /events` |
| **Audit** | [CRITICAL_SIGNATURE_PERSISTENCE_INVESTIGATION.md](audits/CRITICAL_SIGNATURE_PERSISTENCE_INVESTIGATION.md) |

- **Root cause:** CLI uses legacy `POST /events` → `submit_event` inserted with `signature=""`, committed, then put the signature only in the JSON response — never `persist_event_signature`. The v1 path already persisted before commit (`bb43af7`); legacy did not.
- **Impact:** Empty `events.signature` for CLI commits; public proof / verify paths that trust the DB column could mis-handle or fail to treat events as properly anchored.
- **Verification:** Live smoke after fix — `event_id=49d0fcdf-66c9-4f41-8e07-ac49c3c37e42`, signature len 128, `proof_status=anchored`, registry `pv_A4FoZX6wgq4NALbUmfXc9C` REGISTERED / VALID. DB signature matches response byte-for-byte.
- **Regression tests:** `tests/legacy_events_signature_persist.rs` (legacy persist exact match; v1 parity; materialization after anchored commit).

### Legacy identity fields bypass

| | |
|---|---|
| **Status** | **Resolved** |
| **Fix** | `c77172e` — reject `identity_key_id` / `identity_signature` / `identity_fingerprint` on legacy `POST /events` |
| **Audit** | [P1_LEGACY_EVENTS_IDENTITY_FIELDS.md](audits/P1_LEGACY_EVENTS_IDENTITY_FIELDS.md) |

- **Root cause:** Legacy handler deserialized `SubmitEventRequest` identity columns and bound them into `INSERT` with no `require_feature(Identity)` and no PoP (`IdentitySigningService`). Only `/v1/events` ran full Identity validation.
- **Impact:** A crafted HTTP client could store unverified identity claims on events via `/events`. Default CLI path was unaffected (CLI sends no identity fields). Severity Medium; High candidate if any client used legacy for Identity.
- **Verification:** Option A — any identity field on legacy → HTTP **400** (`IdentityNotSupportedOnLegacyPath`); no insert. Identity commits remain only via `POST /v1/events`.
- **Regression tests:** `tests/legacy_events_identity_reject.rs` (legacy + identity rejected; legacy without identity unchanged; v1 valid identity unchanged) + unit check in `src/api/events.rs`.

---

## Overall verdict

**READY FOR CONTROLLED PILOT**

Justification:

- Stage 11.x security / subscription / webhook / smoke gates passed with Critical/High product issues closed or accepted as documented limitations.
- Stage 12.0 Critical gap C1 (no signing-key backup) was **remediated and restore-drilled**, then documented.
- Critical signature persistence gap on CLI `/events` was **diagnosed and fixed** (`8194f6c`) and E2E-verified before real pilot proofs.
- P1 identity bypass on legacy `/events` was **closed** (`c77172e`) without duplicating Identity validation onto the legacy path.
- Perimeter audit: **no unresolved Critical/High** dual-path discrepancies; residual P2/P3 are Low–Medium and documented.
- Operator runbooks exist for deploy, key ops, rollback, onboarding, and manual monitoring.
- Pilot should start on **free** evidence path, with known paid/Identity/TSA and billing limitations disclosed to the user.

Next step is **not** further feature development: create the first real pilot account and observe using [MANUAL_MONITORING.md](MANUAL_MONITORING.md) and [PILOT_ONBOARDING_RUNBOOK.md](PILOT_ONBOARDING_RUNBOOK.md). After retaining the smoke artifact IDs above, pilot DB ledger tables may be truncated (accounts/auth/billing untouched).
