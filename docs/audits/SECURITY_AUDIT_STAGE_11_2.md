# Security Audit Stage 11.2

Date: 2026-07-23

Scope: pre-pilot security & runtime audit after Stage 10 CLI hardening and Stage 11.1 deployment blockers.  
Constraints respected: no architecture / billing-flow / auth-model / Identity / schema / endpoint / CLI changes beyond a minimal production signing-key fail-closed guard.

---

## Summary

Secrets hygiene is sound for pilot: `.env` is gitignored, `.env.example` uses placeholders, no live Paddle keys / PEM private keys / `signing_key.bin` found in tracked tree or sampled git history (`gitleaks` not installed).

Auth/session and API-key controls are strong (hashed sessions, logout invalidation, account-scoped dashboard, hashed API keys + revoke).

Billing webhooks verify HMAC fail-closed and ignore unknown events; v1 subscription enforcement blocks `past_due` writes. Two **High** gaps remain for paid traffic: legacy `/events`/`/chains` skip subscription middleware, and `failed` webhook rows cannot be successfully retried.

**Critical closed in this stage:** production refused to start without `SIGNING_KEY_PATH`, and refuses to auto-create a missing signing key file.

Fresh database: `sqlx migrate run` on empty Postgres applies all 25 migrations and creates the expected schema (verified 2026-07-23).

Full clean pilot path (register → commit → Paddle sandbox upgrade → webhook) was **not** end-to-end executed in this audit window; migrations + runtime guards + code review cover the prerequisites. Recommend one manual sandbox dry-run before inviting the first external user.

---

## Critical

### C1. Production signing key auto-create / missing path — **CLOSED**

| | |
|--|--|
| **Was** | Docs required `SIGNING_KEY_PATH` in production, but unset path fell back to CWD `signing_key.bin` and missing files were auto-created (WARNING only). |
| **Risk** | Fresh deploy or wrong working directory silently minted a new Ed25519 key → proof signature discontinuity. |
| **Fix** | `ENVIRONMENT=production` without `SIGNING_KEY_PATH` → panic. Missing file at that path in production → panic (no auto-create). Dev fallback + create-with-WARNING unchanged. |
| **Evidence** | `src/config.rs`, `src/main.rs`, `docs/DEPLOYMENT.md` |

---

## High

### H1. `past_due` (and related) subscription enforcement bypass on legacy write routes — **CLOSED (Stage 11.3)**

| | |
|--|--|
| **Was** | `subscription_enforcement_middleware` mounted only under `/v1/*`. Legacy `POST /events`, `POST /chains` checked quotas but not `past_due`. |
| **Now** | Legacy `/events` and `/chains` use the same middleware; past_due → `402 payment_required` matching `/v1`. |
| **Evidence** | [STAGE_11_3_SUBSCRIPTION_ENFORCEMENT.md](STAGE_11_3_SUBSCRIPTION_ENFORCEMENT.md), `tests/subscription_enforcement.rs` |

### H2. Failed Paddle webhooks cannot be successfully retried

| | |
|--|--|
| **Problem** | On apply failure the row is marked `failed`. Later Paddle delivery hits `find_by_paddle_event_id` early; `handle_existing_event` only treats `processed` / `waiting_for_account_link` as idempotent → `InvalidStatusTransition` → HTTP 500. |
| **Impact** | Permanent billing desync (missed cancel / `past_due` / plan change) with endless Paddle retries. |
| **Evidence** | `src/paddle/processor.rs` (`handle_existing_event`), `src/paddle/webhook_store.rs` (`mark_processing` allows `failed` but is unreachable after early return), `src/api/paddle_webhook.rs` |
| **Recommendation** | On retry of `failed` (and stuck `received`/`processing`), re-enter processing path; keep idempotency for `processed`. |

---

## Medium

### M1. No webhook timestamp skew / freshness check

HMAC binds `ts` + body (`src/paddle/signature.rs`) but does not reject stale `ts`. Unseen signed payloads remain valid indefinitely until first successful process. Duplicates of known `event_id` are idempotent.

### M2. `ENVIRONMENT` defaults to `development`

Unset `ENVIRONMENT` → `development` (`src/config.rs`). Combined with accidental `DEV_MODE` / `APP_ENV=development`, Secure cookies and tariff switcher can be wrong on a “production-ish” host. Guard only fires when `ENVIRONMENT=production` **and** DEV_MODE is on.

### M3. Non-v1 surfaces skip `past_due` middleware

`/backup/*` and `/accounts/identity/keys` rely on plan entitlements, not the subscription enforcement layer. Past-due paid accounts may retain some paid-adjacent capabilities until plan flags change.

### M4. Weak password policy

Minimum length 8 only (`src/auth/password.rs`). Hashing is Argon2id (good). Tighten for public signup if pilot is multi-tenant.

### M5. `PADDLE_API_BASE_URL` defaults to live API

Unset base URL → `https://api.paddle.com`. Sandbox hosts must set sandbox URL explicitly (documented).

### M6. Docker Compose local DB credentials

`docker-compose.yml` uses `ledger`/`ledger` — local-only; do not reuse for pilot/production.

---

## Low

### L1. Session cookie `SameSite=Lax` (not `Strict`)

HttpOnly + Path=/ + Secure (when not DEV_MODE). Lax is acceptable; HTMX mutations add Origin/Referer checks.

### L2. API key hash is unsalted SHA-256

Acceptable given high-entropy secrets; not comparable to password hashing. Prefix + revoke path present.

### L3. Login revokes sibling sessions

`create_session` deletes other sessions for the account — intentional single-session UX.

### L4. Empty / near-empty historical migration

`migrations/20260628202402_add_sequence_to_events.sql` is tiny (~29 bytes); a later migration applies the real change. Do not rewrite applied history.

### L5. Listen address hardcoded `:3000`

Front with reverse proxy for pilot; `PORT` later.

---

## Deferred

| Item | Note |
|------|------|
| `/health` / readiness | Add before monitoring/orchestration (Stage 11.1). |
| In-process migration automation | Keep ops step: configure → `sqlx migrate run` → start. |
| Webhook timestamp tolerance | After H2; align with Paddle guidance. |
| Legacy route deprecation | Prefer closing H1 via middleware or production disable. |
| gitleaks in CI | Tool not present locally; add to CI when available. |
| Full clean pilot E2E (register → Paddle sandbox upgrade → webhook) | Manual dry-run recommended before first external user. |

---

## Verified Controls

### Secrets & access

| Control | Result |
|---------|--------|
| `.env` tracked in git | **No** (gitignored) |
| `.env.example` placeholders only | **Yes** (`pdl_*_replace_me`, `test_replace_me`) |
| `signing_key.bin` / `*.pem` / `*.key` ignored | **Yes** |
| Live Paddle keys / DB passwords / PEMs in tracked sources | **None found** |
| `signing_key.bin` in git history | **None found** |
| Docker image embeds secrets | **No** (runtime env expected) |

### Runtime fail-fast

| Variable / scenario | Behavior |
|---------------------|----------|
| Missing `DATABASE_URL` (not supplied by `.env`) | Panic: `DATABASE_URL must be set` / DB connect failure |
| Missing Paddle secrets (non-test) | Panic at `AppConfig::from_env` |
| `ENVIRONMENT=development` + `DEV_MODE=true` | Starts; prints Dev mode enabled |
| `ENVIRONMENT=production` + `DEV_MODE=true` | Panic: `DEV_MODE cannot be enabled in production environment` |
| `ENVIRONMENT=production` + unset `SIGNING_KEY_PATH` | Panic: `SIGNING_KEY_PATH must be set in production environment` (**new**) |
| `ENVIRONMENT=production` + missing key file | Panic: refuses auto-create (**new**) |
| `SIGNING_KEY_PATH` unset in development | CWD `signing_key.bin` fallback |
| Note | `dotenvy` loads `.env` into missing vars — shell “unset” tests must clear or override `.env` |

### Authentication & authorization

| Control | Evidence |
|---------|----------|
| Cookie: HttpOnly, SameSite=Lax; Secure when not DEV_MODE | `src/auth/session_store.rs` |
| Session token hashed at rest (SHA-256); 256-bit entropy | `session_store.rs` |
| Logout deletes server session + clears cookie | `web_auth.rs`, tests |
| Dashboard API without session → 401; UI → `/login` | session middlewares |
| API keys hashed; plaintext only at creation response | `api_key.rs`, dashboard create |
| Revoke sets `revoked_at`; cross-account revoke → NotFound | `accounts` service + tests |
| Dashboard uses `session.account_id` only | dashboard API/UI |
| Event IDOR → 404 for foreign events | `api/v1/event_access.rs` |

### Billing

| Control | Evidence |
|---------|----------|
| Webhook HMAC fail-closed before parse/DB | `api/paddle_webhook.rs`, `paddle/signature.rs` |
| Unknown event types → 200 `ignored` | `processor.rs` |
| Idempotency by `paddle_event_id` + payload hash | `processor.rs`, store |
| Out-of-order skip by `occurred_at` | `processor.rs` |
| v1 `past_due` blocks writes, allows reads | `subscription_enforcement` + tests |

### Database & migrations

| Check | Result |
|-------|--------|
| Required order | configure env → `sqlx migrate run` → start (`docs/DEPLOYMENT.md`) |
| Fresh DB migrate | **Verified** on empty `evident_ledger_pilot_11_2` (all migrations applied, schema present) |
| App auto-migrates | **No** (deferred / ops-required) |

---

## Pilot readiness verdict

**Safe for a tightly controlled first pilot** if:

1. `ENVIRONMENT=production`, `DEV_MODE` off, `SIGNING_KEY_PATH` set to a **backed-up** key file that already exists.
2. Paddle sandbox/live secrets and `PADDLE_API_BASE_URL` match the intended catalog.
3. Migrations applied before start on a dedicated database.
4. Operator understands **H1/H2**: prefer `/v1` for writes; monitor webhook `failed` rows until H2 is fixed; do not rely on legacy CLI writers for past-due enforcement.

**Before opening paid multi-user pilot:** close **H1** and **H2**.

---

## Changes shipped with this audit

- Production `SIGNING_KEY_PATH` required + no auto-create of missing production keys.
- Deployment docs updated to match enforcement.
- This report: `docs/audits/SECURITY_AUDIT_STAGE_11_2.md`.
