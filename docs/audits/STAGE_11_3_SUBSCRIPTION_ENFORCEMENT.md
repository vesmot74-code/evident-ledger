# Stage 11.3 Subscription Enforcement Audit

Date: 2026-07-23

Closes **H1** from [SECURITY_AUDIT_STAGE_11_2.md](SECURITY_AUDIT_STAGE_11_2.md): legacy `/events` and `/chains` write paths now use the same `subscription_enforcement_middleware` as `/v1/*`.

---

## Checked endpoints

| Endpoint | Method | Mutates ledger / account state? | Subscription protected? | Notes |
|---|---|---|---|---|
| `/events/` | POST | Yes (append event) | **Yes** | Same middleware as `/v1` |
| `/chains/` | POST | Yes (create chain) | **Yes** | Only write under `/chains` (no rename/delete routes) |
| `/v1/events/` | POST | Yes | **Yes** | Pre-existing (Stage 8.2c) |
| `/v1/identity/keys/:id/revoke` | POST | Yes (identity) | **Yes** | Under `/v1` layer |
| `/v1/proof/*`, `/v1/verify/*`, `/v1/account/*` | GET | No | N/A (reads allowed when `past_due`) | Middleware skips payment block on reads |
| `/verify/:chain_id`, `/verify/proof/*`, attestation | GET | No | N/A | Read/verify |
| `/verify/hash` | POST | No (hash check only) | No | Not a ledger write |
| `/identity/` | GET | No | N/A | Public key disclosure |
| `/account/usage`, `/capabilities`, `/key-status` | GET | No | No | Read-only account info |
| `/account/dev/change-plan` | POST | Yes (dev only) | No | Gated by `DEV_MODE`; not a paid commit path |
| `/backup/create` | POST | Yes (backup artifact) | **No** | Plan entitlement only; Stage 11.2 **M3** — out of H1 scope |
| `/backup/list`, `/:id`, download | GET | No | No | Reads + entitlement |
| `/accounts/register` | POST | Yes (new account) | N/A | Unauthenticated signup |
| `/accounts/api-keys` | POST/DELETE | Yes (API keys) | No | Account admin; not evidence commit |
| `/accounts/identity/keys/*` | POST | Yes (identity keys) | **No** | Entitlement `require_feature`; Stage 11.2 **M3** |
| `/dashboard/*` API writes | POST/DELETE | Yes (session) | Session auth | Billing upgrade uses Paddle; not legacy CLI bypass |
| `/paddle/webhook` | POST | Yes (billing) | Signature auth | Must remain reachable when past_due |

### `/chains` sub-operations

Only `POST /chains/` (create) exists. No rename, delete, or update routes under `/chains`.

### `/events` sub-operations

Only `POST /events/` (submit/append) exists on the legacy router.

---

## Behavior matrix

Same rules as `/v1/*` via `write_blocked_by_subscription` + usage limit on writes:

| Status | Legacy write (`/events`, `/chains`) |
|---|---|
| free (within limits) | Allowed |
| free (over monthly commit limit) | `429 usage_limit_exceeded` |
| active | Allowed |
| canceled + `current_period_end` > now | Allowed |
| past_due | `402 payment_required` (same payload as `/v1`) |
| canceled + expired | Lazy transition → free / `none`, then free-tier rules |

---

## Changes

- `src/api/events.rs`, `src/api/chains.rs`: layer existing `subscription_enforcement_middleware` after `api_key_auth_middleware` (inserts `AuthedAccount` for the guard; preserves legacy auth error shape).
- `src/auth/mod.rs`: shared `api_key_auth_middleware`.
- No Paddle / webhook / schema / tariff / CLI changes.

---

## Tests

In `tests/subscription_enforcement.rs`:

- `legacy_events_paid_active_write_passes`
- `legacy_events_past_due_matches_v1_payment_required` (status + error code/message parity)
- `legacy_events_free_account_write_allowed`
- `legacy_events_canceled_before_period_end_write_passes`
- `legacy_chains_past_due_returns_payment_required`
- `legacy_chains_active_write_passes`

Run: `cargo test --test subscription_enforcement`
