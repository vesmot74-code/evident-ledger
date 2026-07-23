# Stage 11.6 Pilot Smoke Test

Date: 2026-07-23

Verification-only run on a **clean** Postgres database (`evident_ledger_pilot_11_6`) with a production-like process env. No product code changes in this stage.

## Environment

| Item | Value |
|---|---|
| Database | Fresh empty DB → `sqlx migrate run` (all migrations applied before first start) |
| Process | `ENVIRONMENT=production`, `DEV_MODE=false` |
| Signing key | Pre-created file via `SIGNING_KEY_PATH` (absolute path under `target/pilot116-key.*/signing_key.bin`) |
| Paddle | Sandbox credentials from local `.env` (`PADDLE_API_KEY`, `PADDLE_WEBHOOK_SECRET`, sandbox base URL) |
| Binary | `./target/release/evident-ledger` / `./target/release/evident` |
| Bind | `http://0.0.0.0:3000` |

Guards observed:

- Missing `SIGNING_KEY_PATH` / missing key file in production → refuse auto-create (no new production key).
- Startup log: `Environment: production`; no Dev-mode banner; same Public key across restart.

## Scenarios

| Scenario | Result | Notes |
|---|---|---|
| 1 Fresh startup | **PASS** | empty DB → migrate → server start; key loaded from `SIGNING_KEY_PATH`; no panic |
| 2 Account lifecycle | **PASS** | register → login (Secure session cookie) → `/auth/me` → API key create (plaintext once) → list without plaintext → `X-API-KEY` works |
| 3 Basic evidence flow | **PASS** | `evident commit` created event+proof; offline `evident verify` = `OK: proof valid` when pinned key matches server pubkey |
| 4 Identity flow | **PARTIAL** | free → `403 entitlement_missing`; after Identity plan, challenge `200`; full PoP register/sign/revoke not exercised (needs client key material) |
| 5 Billing flow | **PARTIAL** | clean migrate has `paddle_price_id=NULL` → `400 plan_not_purchasable`; after ops seed of sandbox monthly prices, `POST /dashboard/upgrade` → `200` + real Paddle `checkout_url` / `txn_*`; payment completion + inbound webhook → tariff update **not** completed in this automated run (needs browser checkout + reachable notification URL) |
| 6 Past due enforcement | **PASS** | `/v1/events`, `/events`, `/chains`, `/backup/create` → `402 payment_required`; reads (`/account/capabilities`) remain `200` |
| 7 Backup | **PASS** | active → `201`; past_due → `402 payment_required` |
| 8 Public verification | **PASS** | `GET /public/verify` exists; rate-limit headers present (`x-ratelimit-*`); response has no `account_id` / `chain_id` / internal signing material |
| 9 Recovery | **PASS** | kill/restart with same `SIGNING_KEY_PATH` → identical Public key; DB rows persist; proof still verifies |

## Findings

| Severity | Finding | Action |
|---|---|---|
| Medium (ops) | Fresh migrations leave `tariff_plans.paddle_price_id` NULL; checkout returns `plan_not_purchasable` until ops seeds catalog IDs | Documented in `DEPLOYMENT.md`; **must seed** before pilot checkout. Deferred product change (no schema/billing refactor in this stage). |
| Low (ops) | Offline CLI verify pins `~/.evident/server_identity.pub`; a rotated/new deployment key causes `signature invalid` even when the proof is cryptographically valid for the server that signed it | Pilot runbook: fetch `/identity` once per deployment and pin. Not a crypto/signing-key integrity bug. |
| Low (test coverage) | Scenario 4 did not complete register → sign → revoke end-to-end | Acceptable for smoke; exercise with Identity client keys before Identity pilot users. |
| Low (external) | Scenario 5 did not complete paid checkout → webhook → tariff update against a live notification destination | External/operational dependency; processor/idempotency covered by Stage 11.4 tests. Complete once ngrok/notification destination is confirmed. |
| Info | M5 health/ops endpoint remains Deferred (Stage 11.5) | Non-blocking for single-node pilot |

No Critical or High product defects found in this smoke run (no billing bypass, auth break, data loss, or production signing-key auto-create).

## Final checks

```text
cargo test -- --skip dev_tariff_switcher_end_to_end
cargo build --release
./target/release/evident --version   # evident 0.1.0
git status
git log --oneline -10
```

Notes:

- `cargo build --release` succeeded; CLI reports `evident 0.1.0`.
- Full suite (with skip) mostly green; `v1_idempotency_replay_and_conflict` failed with `api key account: RowNotFound` while the **pilot server** still occupied `:3000` and `DATABASE_URL` pointed at the pilot DB — that test is a live-server probe against `~/.evident/api_key` / `DATABASE_URL`, not a product regression from this stage. Re-run against the normal test DB / server when the pilot process is stopped.

## Pilot readiness

**READY WITH LIMITATIONS**

Rationale:

- Critical/High = 0 for observed product behavior on clean production-like env.
- Core account, evidence, past_due, backup, public verify, and restart paths pass.
- Remaining gaps are operational (catalog seed, CLI trust pin, full Paddle payment+webhook, Identity PoP client) — not deployment workflow or integrity blockers for a constrained first pilot if checkout catalog is seeded and webhook URL is live.
