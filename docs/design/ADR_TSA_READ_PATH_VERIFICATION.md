# ADR: TSA read-path verification cache

## Status

Accepted (2026-07-25)

## Context

Write-path TSA stamping already validates RFC3161 tokens via
`notary_tsa::parse_and_validate_tsr` before insert. Read-path
(`resolve_proof_state`) previously recognized only Evident JSON stub tokens
and skipped cryptographic checks for FreeTSA DER tokens, so proof responses
could not independently prove token binding or TSA signature validity.

## Decision

Hybrid A+B:

1. **Source of truth** — cryptographic verification on first read / cache miss:
   - message-imprint via `parse_and_validate_tsr`
   - OpenSSL `ts -verify` against the same FreeTSA CA bundle
     (`FREETSA_CA_CERT_PATH` / `FREETSA_UNTRUSTED_CERT_PATH`) used by
     `vendor/notary-tsa` smoke tests
2. **Cache** — store `verification_status`, `verified_at`, `token_sha256` on
   `tsa_tokens`. Reuse as `verified_cached` only when
   `token_sha256` matches the current token bytes and status is `verified`.
3. **Race** — `UPDATE … WHERE verification_status IS NULL OR token_sha256 IS DISTINCT FROM $sha`
   so parallel readers do not thrash the cache after the first successful write.
4. **Stub enforcement** — Evident JSON stubs (`"stub":true`) verify only when
   `DEV_MODE=true` or `APP_ENV=development`; otherwise `failed`.

API `tsa.verification_status`: `verified` | `verified_cached` | `failed` | `unavailable`.

## Known limitation

`verified_cached` reflects the outcome at `verified_at` against the CA bundle
available at that time. Automatic re-verification when a TSA certificate is
later revoked is **not** performed. Operators must invalidate or re-check
explicitly if trust material changes.

## Consequences

- Proof read paths may invoke OpenSSL on cache miss (latency).
- Missing CA files yield `unavailable` (imprint may still have succeeded);
  they do not silently mark the token verified.
- Existing integration tests that seed stub tokens require
  `APP_ENV=development` or `DEV_MODE=true`.
