# Pilot Readiness Summary

Stage 12.0 — operations package for a **controlled** first pilot. Not a product feature release.

Security / smoke lineage:

- [audits/SECURITY_AUDIT_STAGE_11_2.md](audits/SECURITY_AUDIT_STAGE_11_2.md) — Critical=0, High=0 (closed blockers)
- [audits/STAGE_11_3_SUBSCRIPTION_ENFORCEMENT.md](audits/STAGE_11_3_SUBSCRIPTION_ENFORCEMENT.md)
- [audits/STAGE_11_4_WEBHOOK_RELIABILITY.md](audits/STAGE_11_4_WEBHOOK_RELIABILITY.md)
- [audits/STAGE_11_5_MEDIUM_FINDINGS.md](audits/STAGE_11_5_MEDIUM_FINDINGS.md) — M3 closed; M4 accepted; M5 deferred
- [audits/STAGE_11_6_PILOT_SMOKE_TEST.md](audits/STAGE_11_6_PILOT_SMOKE_TEST.md) — READY WITH LIMITATIONS
- [audits/STAGE_12_0_FINDINGS.md](audits/STAGE_12_0_FINDINGS.md) — C1 signing-key backup **Closed**
- [audits/CRITICAL_SIGNATURE_PERSISTENCE_INVESTIGATION.md](audits/CRITICAL_SIGNATURE_PERSISTENCE_INVESTIGATION.md) — empty `events.signature` on CLI `/events` **Fixed**

| Area | Document | Status |
|---|---|---|
| Deployment | [PILOT_DEPLOYMENT_CHECKLIST.md](PILOT_DEPLOYMENT_CHECKLIST.md) | Ready |
| Signing key ops | [SIGNING_KEY_OPERATIONS.md](SIGNING_KEY_OPERATIONS.md) | Ready (backup + restore drill 2026-07-23) |
| Rollback | [ROLLBACK_PROCEDURE.md](ROLLBACK_PROCEDURE.md) | Ready |
| Onboarding | [PILOT_ONBOARDING_RUNBOOK.md](PILOT_ONBOARDING_RUNBOOK.md) | Ready |
| Monitoring | [MANUAL_MONITORING.md](MANUAL_MONITORING.md) | Ready (interim; no `/health`) |
| SLA draft | [PILOT_SLA_DRAFT.md](PILOT_SLA_DRAFT.md) | DRAFT — internal review required |
| Stage 12 findings | [audits/STAGE_12_0_FINDINGS.md](audits/STAGE_12_0_FINDINGS.md) | C1 Closed |
| Signature persist (CLI) | [audits/CRITICAL_SIGNATURE_PERSISTENCE_INVESTIGATION.md](audits/CRITICAL_SIGNATURE_PERSISTENCE_INVESTIGATION.md) | **Closed** — legacy `/events` persists before commit |

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
11. ~~**CLI `/events` left `events.signature` empty**~~ — **Closed**: legacy path now persists via `persist_event_signature` before commit (same model as `/v1/events`). See critical investigation audit.

---

## Overall verdict

**READY FOR CONTROLLED PILOT**

Justification:

- Stage 11.x security / subscription / webhook / smoke gates passed with Critical/High product issues closed or accepted as documented limitations.
- Stage 12.0 Critical gap C1 (no signing-key backup) was **remediated and restore-drilled**, then documented.
- Critical signature persistence gap on CLI `/events` was **diagnosed and fixed** before real pilot proofs (test DB rows are disposable; truncate after smoke).
- Operator runbooks exist for deploy, key ops, rollback, onboarding, and manual monitoring.
- Pilot should start on **free** evidence path, with known paid/Identity/TSA and billing limitations disclosed to the user.

Next step is **not** further feature development: create the first real pilot account and observe using [MANUAL_MONITORING.md](MANUAL_MONITORING.md) and [PILOT_ONBOARDING_RUNBOOK.md](PILOT_ONBOARDING_RUNBOOK.md).
