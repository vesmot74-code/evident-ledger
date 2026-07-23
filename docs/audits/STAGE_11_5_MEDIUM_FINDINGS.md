# Stage 11.5 Medium Findings Audit

Date: 2026-07-23

## Findings

| Finding | Status | Decision |
|---|---|---|
| M3 Backup | **Closed** | Existing `subscription_enforcement_middleware` on `/backup` |
| M4 Identity | **Accepted** | `/accounts/*` account-management exception (BILLING_MODEL) |
| M5 Health / ops | **Deferred** | Before monitoring/orchestration; no new endpoints |

Other Medium from 11.2 (timestamp skew, ENVIRONMENT default, password policy, Paddle URL default, docker-compose creds): **Accepted** or **Deferred** as in the review — not pilot blockers.

---

## Changes

### Code

- `src/api/backup.rs` — layer `api_key_auth_middleware` + `subscription_enforcement_middleware` (same pattern as Stage 11.3 legacy writes).
- Middleware/service comments updated to include `/backup`.

### Not changed

- Backup format / export / storage path logic
- DB schema / migrations
- Paddle / webhook / Identity protocol
- Auth/session model

---

## Documentation Alignment

| Doc | Update |
|---|---|
| `SECURITY.md` invariant 18 + §2.5 note | `past_due` blocks **paid write capabilities**; documents `/accounts/*` exception |
| `docs/BILLING_MODEL.md` §5 | Explicit row/table: `POST /backup/create` → 402; identity keys → allowed |
| This audit | Records Closed / Accepted / Deferred |

SECURITY.md and BILLING_MODEL.md are aligned: paid writes (ledger + backup create) blocked on `past_due`; identity under `/accounts` remains available.

---

## Tests

In `tests/subscription_enforcement.rs`:

| Test | Expectation |
|---|---|
| `backup_create_active_subscription_passes` | `vault` + `active` → `201 created` |
| `backup_create_past_due_matches_v1_payment_required` | `402` + same payload as `/v1`; no backup row |
| `backup_create_free_account_keeps_entitlement_behavior` | `403 feature_not_available` (unchanged) |

---

## Pilot posture

```
H1 ✅  H2 ✅  M3 ✅
M4 Accepted  M5 Deferred
```

Next: Stage 11.6 — Pilot Smoke Test.
