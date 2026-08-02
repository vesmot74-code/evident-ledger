# Startup exit codes

Categorical exit codes for the Evident Ledger server binary (`src/main.rs` and startup helpers in `src/config.rs` / `src/db.rs` / `src/signing.rs`).

| Code | Category | Scope |
|------|----------|-------|
| 1 | TSA_TRUST_ERROR | TSA trust validation failure at startup (Stage 13.7) |
| 2 | SIGNING_ERROR | Server signing key missing (production) or `ServerSigner::load_or_create` failure |
| 3 | DATABASE_ERROR | `DATABASE_URL` missing or database connection failure |
| 4 | CONFIG_ERROR | Configuration validation failures (`DEV_MODE`+production, `SIGNING_KEY_PATH`, Paddle secrets/tokens) |
| 5 | NETWORK_ERROR | TCP listen bind failure |
| 27 | SERVER_RUNTIME_ERROR | `axum::serve` runtime failure after successful bind |

## Category purpose

- **TSA_TRUST_ERROR (1)** — production-hard failure when FreeTSA trust material cannot be validated. Owned by Stage 13.7 (`tsa::enforce_tsa_trust_at_startup`). **This refactor does not change exit code 1 or TSA trust validation behavior.**
- **SIGNING_ERROR (2)** — cryptographic identity of the server cannot be established (missing production key path, unreadable/invalid key, or inability to create/persist a new key).
- **DATABASE_ERROR (3)** — process cannot obtain a usable Postgres pool at startup.
- **CONFIG_ERROR (4)** — environment/configuration is invalid before runtime services start. Message text stays specific; the exit code is the shared category.
- **NETWORK_ERROR (5)** — the process cannot bind its listen address.
- **SERVER_RUNTIME_ERROR (27)** — the HTTP server fails after bind. This is a runtime failure (prefix `SERVER_RUNTIME_ERROR:`), not a `STARTUP_ERROR`.

## Separate binary: `bin/verify.rs`

`src/bin/verify.rs` is a different binary with its own exit-code space. Numeric overlap with the server table above is allowed and is not a bug. Do not unify or “fix” verify exit codes against this document.
