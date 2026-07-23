# Deployment Findings — Stage 11.1

Status after pre-pilot blocker closure.

Severity legend: **High** / **Medium** / **Low**.

---

## Closed

### Cargo.lock tracked

| | |
|--|--|
| **Was** | `.gitignore` excluded `Cargo.lock` → non-reproducible builds. |
| **Now** | Root `Cargo.lock` is tracked; removed from `.gitignore`. |
| **Verify** | `git ls-files Cargo.lock` and `cargo build --release`. |

### DEV_MODE production guard

| | |
|--|--|
| **Was** | `DEV_MODE=true` could be left on in production (insecure cookies + tariff switcher). |
| **Now** | `ENVIRONMENT=production` + `DEV_MODE` (or `APP_ENV=development`) → startup panic: `DEV_MODE cannot be enabled in production environment`. |
| **Verify** | `DEV_MODE=true` + `ENVIRONMENT=development` starts; + `ENVIRONMENT=production` fails. |

### SIGNING_KEY_PATH

| | |
|--|--|
| **Was** | Key always at CWD-relative `signing_key.bin`. |
| **Now** | `SIGNING_KEY_PATH` selects the file exactly when set; unset keeps `./signing_key.bin` fallback. New keys log a WARNING with full path. **Required for production** (documented). |
| **Verify** | Unset path → CWD fallback; set path → that file only (no silent fallback). |

---

## Deferred

### No dedicated health / readiness endpoint

| | |
|--|--|
| **Status** | **Deferred** |
| **Problem** | No `/health`, `/ready`, or `/live` route. |
| **Note** | Add before monitoring/orchestration integration. Until then use checks in [DEPLOYMENT.md](DEPLOYMENT.md#health-check). |

### Migrations not applied on application startup

| | |
|--|--|
| **Status** | **Deferred** (ops step remains) |
| **Problem** | Process does not run `sqlx migrate` itself. |
| **Note** | `sqlx migrate run` is a **REQUIRED** deployment step (configure → migrate → start). Automation may come later; do not auto-migrate in-process without an explicit decision. |

---

## Open (non-blocking for pilot)

### Listen port is not configurable

| | |
|--|--|
| **Problem** | Bind address hardcoded to `0.0.0.0:3000`. |
| **Recommendation** | Front with a reverse proxy; add `PORT` / `BIND_ADDR` later. |

### Empty migration file in history

| | |
|--|--|
| **Problem** | `migrations/20260628202402_add_sequence_to_events.sql` is empty. |
| **Recommendation** | Leave as-is (do not rewrite applied migration history). |

### Machine TSA endpoint is hardcoded

| | |
|--|--|
| **Problem** | FreeTSA URL is a code constant. |
| **Recommendation** | Acceptable for pilot Machine TSA; configurable providers before regulated claims. |

---

## Secrets scan summary

| Check | Result |
|-------|--------|
| `.env` tracked in git? | **No** (gitignored) |
| Signing key files tracked? | **No** (`*.bin` / `signing_key.bin` gitignored) |
| Live Paddle secrets / private keys in tree? | **None found** in tracked sources |
| `Cargo.lock` tracked? | **Yes** (closed) |
| Production `SIGNING_KEY_PATH` enforced? | **Yes** (Stage 11.2 — see [SECURITY_AUDIT_STAGE_11_2.md](audits/SECURITY_AUDIT_STAGE_11_2.md)) |

---

## Startup panics (expected fail-fast)

- Missing `DATABASE_URL` or DB connect failure
- Missing `PADDLE_WEBHOOK_SECRET`, `PADDLE_API_KEY`, or `PADDLE_CLIENT_TOKEN` outside test builds
- `DEV_MODE` enabled while `ENVIRONMENT=production`
- Failure to bind `:3000`
