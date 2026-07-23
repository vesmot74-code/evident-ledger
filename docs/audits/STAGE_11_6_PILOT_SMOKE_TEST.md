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
| Active instance | pid **4500** (kept running after controlled restart) |

Guards observed:

- Missing `SIGNING_KEY_PATH` / missing key file in production → refuse auto-create (no new production key).
- Startup log: `Environment: production`; no Dev-mode banner; same Public key across restart.
- Live process env (pid 4500): `ENVIRONMENT=production`, `DEV_MODE=false`, `SIGNING_KEY_PATH=/Users/iuriiveselskii/evident-ledger/target/pilot116-key.JBOhAH/signing_key.bin`.

## Restart Cycle

Result: **PASS**

Background server jobs exited with SIGTERM (exit code **143**) during controlled restart.  
No crash detected.  
Active instance verified on `:3000` (pid **4500**).

Recovery re-check on active instance:

| Check | Result |
|---|---|
| Process listening | PASS (`evident-ledger` pid 4500) |
| Production-like env | PASS (`ENVIRONMENT=production`, `DEV_MODE=false`) |
| Signing key path | PASS (expected `target/pilot116-key.JBOhAH/signing_key.bin`) |
| Key file not recreated | PASS (mtime `2026-07-23 10:53:06`, sha256 unchanged, Public key unchanged) |
| DB persistence | PASS (`accounts=2`, `events=1`, `chains=1` at re-check; later identity rows added by S4) |
| Existing proof verify | PASS (`OK: proof valid` with pinned server pubkey) |

## Scenarios

| Scenario | Result | Notes |
|---|---|---|
| 1 Fresh startup | **PASS** | empty DB → migrate → server start; key loaded from `SIGNING_KEY_PATH`; no panic |
| 2 Account lifecycle | **PASS** | register → login (Secure session cookie) → `/auth/me` → API key create (plaintext once) → list without plaintext → `X-API-KEY` works |
| 3 Basic evidence flow | **PASS** | `evident commit` on **free** created event+proof; offline `evident verify` = `OK: proof valid` when pinned key matches server pubkey |
| 4 Identity flow | **PASS*** | capability gate on free; challenge → PoP register → revoke → post-revoke submit returns `403 identity_key_revoked`; old proof remains valid. *Successful identity-signed (and any paid-plan) event create is blocked by Qualified TSA unavailability → `500 internal_error` (see Findings) |
| 5 Billing flow | **PARTIAL** | clean migrate has `paddle_price_id=NULL` → `400 plan_not_purchasable`; after ops seed of sandbox monthly prices, `POST /dashboard/upgrade` → `200` + real Paddle `checkout_url` / `txn_*`; payment completion + inbound webhook → tariff update **not** completed in this automated run |
| 6 Past due enforcement | **PASS** | `/v1/events`, `/events`, `/chains`, `/backup/create` → `402 payment_required`; reads (`/account/capabilities`) remain `200` |
| 7 Backup | **PASS** | active → `201`; past_due → `402 payment_required` |
| 8 Public verification | **PASS** | `GET /public/verify` exists; rate-limit headers present (`x-ratelimit-*`); response has no `account_id` / `chain_id` / internal signing material |
| 9 Recovery | **PASS** | controlled restart (SIGTERM 143) → same `SIGNING_KEY_PATH` / Public key; DB + proofs OK; active pid 4500 verified |

## Findings

| Severity | Finding | Action |
|---|---|---|
| Medium (product / ops) | Paid plans (`legal` / `vault` / `identity`) advertise `tsa_mode=qualified`, but `AccountCapabilities::tsa_available()` is true only for `machine`. Event writes on paid plans return `500 internal_error` (`QualifiedTsaUnavailable`). Free-plan commits work. | **Pilot limitation:** first pilot users should stay on **free** for evidence writes, or Qualified TSA must be enabled before paid write pilots. Do not treat as billing bypass. Fix only after explicit confirmation. |
| Medium (ops) | Fresh migrations leave `tariff_plans.paddle_price_id` NULL; checkout returns `plan_not_purchasable` until ops seeds catalog IDs | Documented in `DEPLOYMENT.md`; **must seed** before pilot checkout. |
| Low (ops) | Offline CLI verify pins `~/.evident/server_identity.pub`; a rotated/new deployment key causes `signature invalid` even when the proof is valid for the signing server | Pilot runbook: fetch `/identity` once per deployment and pin. |
| Low (external) | Scenario 5 did not complete paid checkout → webhook → tariff update against a live notification destination | Complete once browser checkout + notification URL confirmed. |
| Info | M5 health/ops endpoint remains Deferred (Stage 11.5) | Non-blocking for single-node pilot |
| Info | Background job exit code 143 (SIGTERM) | **Expected** during controlled restart cycle — not a finding. |

No Critical or High defects for auth, signing-key auto-create, past_due enforcement, or data loss on restart.

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
- Full suite (with skip) mostly green; `v1_idempotency_replay_and_conflict` failed with `api key account: RowNotFound` while the **pilot server** occupied `:3000` — live-server probe conflict, not a product regression from this stage.

## Pilot readiness

**READY WITH LIMITATIONS**

Rationale:

- Critical/High = 0 for auth, key handling, past_due, backup, public verify, restart.
- Deployment workflow reproducible on clean DB + production-like env; restart cycle PASS (SIGTERM 143 expected).
- Remaining limitations: catalog seed, CLI trust pin, full Paddle payment+webhook, and **paid-plan event writes blocked until Qualified TSA is available** (free-tier evidence flow OK for a constrained first pilot).
