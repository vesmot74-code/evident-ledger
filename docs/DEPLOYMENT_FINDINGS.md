# Deployment Findings — Stage 11.1

Audit-only findings for first pilot / production deployment.  
**No automatic remediations were applied** beyond documenting missing env vars in `.env.example`.

Severity legend: **High** / **Medium** / **Low**.

---

## 1. `Cargo.lock` is gitignored

| | |
|--|--|
| **Problem** | `.gitignore` excludes `Cargo.lock`, so dependency versions are not pinned in the repository. |
| **Risk** | Non-reproducible release builds; “works on my machine” drift between pilot rebuilds. |
| **Recommendation** | For the application crate, **track `Cargo.lock`** in git (standard for binaries). Decide in a follow-up change; do not silently force-add without team agreement. |

---

## 2. No dedicated health / readiness endpoint

| | |
|--|--|
| **Problem** | No `/health`, `/ready`, or `/live` route. |
| **Risk** | Load balancers and orchestrators cannot probe liveness without scraping HTML or inventing heuristics. |
| **Recommendation** | Add a minimal authenticated-or-public health route in a later stage. Until then use checks in [DEPLOYMENT.md](DEPLOYMENT.md#health-check). |

---

## 3. Migrations are not applied on application startup

| | |
|--|--|
| **Problem** | `evident-ledger` connects to Postgres but never runs `sqlx migrate`. |
| **Risk** | Fresh deploy with empty DB fails at runtime when tables are missing; easy to forget `sqlx migrate run`. |
| **Recommendation** | Keep migrate-as-ops step for now (documented). Optionally add an explicit migrate job in CI/CD later — not auto-migrate-in-process without a decision. |

---

## 4. `DEV_MODE` / `APP_ENV=development` must stay off in production

| | |
|--|--|
| **Problem** | When enabled: (1) `POST /account/dev/change-plan` allows tariff switching; (2) session cookies are issued **without** the `Secure` flag. |
| **Risk** | Privilege / billing bypass in production; session cookies over plain HTTP if TLS is misconfigured. |
| **Recommendation** | Ensure production env has neither `DEV_MODE=true` nor `APP_ENV=development`. Alert if startup log contains `Dev mode: enabled`. |

---

## 5. Server signing key path is CWD-relative

| | |
|--|--|
| **Problem** | `ServerSigner::load_or_create("signing_key.bin")` uses the process working directory. |
| **Risk** | Restart from a different CWD creates a **new** key → new public key, historical verification continuity issues; key may be left on disk unprotected if permissions wrong. |
| **Recommendation** | Run under a fixed service directory; back up `signing_key.bin`; later add an explicit `SIGNING_KEY_PATH` env (follow-up). File is gitignored — good. |

---

## 6. Listen port is not configurable

| | |
|--|--|
| **Problem** | Bind address is hardcoded to `0.0.0.0:3000`. |
| **Risk** | Port conflicts; awkward multi-instance or restricted hosts without a proxy rewrite. |
| **Recommendation** | Front with a reverse proxy for pilot; add `PORT` / `BIND_ADDR` in a later hardening stage. |

---

## 7. Empty migration file in history

| | |
|--|--|
| **Problem** | `migrations/20260628202402_add_sequence_to_events.sql` is empty; the real `sequence` change is in `20260628202432_…`. |
| **Risk** | Low — confusing for operators reading migration list; no schema damage observed. |
| **Recommendation** | Leave as-is (do not rewrite applied migration history). Document only. |

---

## 8. Machine TSA endpoint is hardcoded

| | |
|--|--|
| **Problem** | FreeTSA URL is a code constant; no env override; qualified TSA is not fully productized. |
| **Risk** | External dependency / availability; not suitable as sole “qualified” evidence path. |
| **Recommendation** | Acceptable for pilot with Machine TSA; plan configurable TSA providers before regulated production claims. |

---

## Secrets scan summary

| Check | Result |
|-------|--------|
| `.env` tracked in git? | **No** (gitignored) |
| `signing_key.bin` tracked? | **No** (gitignored) |
| Live Paddle secrets / private keys in tree? | **None found** in tracked sources |
| Test-only password strings in unit tests? | Present (`src/auth/password.rs`, `tests/web_auth.rs`) — **not** production credentials |

No secret scrubbing was performed (none required for tracked files).

---

## Startup panics (expected fail-fast)

Documented intentional hard failures:

- Missing `DATABASE_URL` or DB connect failure
- Missing `PADDLE_WEBHOOK_SECRET`, `PADDLE_API_KEY`, or `PADDLE_CLIENT_TOKEN` outside test builds
- Failure to bind `:3000`

These are safer than silent misconfiguration for a pilot, but operators must set env before launch.
