# Stage 13.7.2 Security Review

Date: 2026-07-31

Commit: `75cfc12`

Branch: `stage-13.7-release-candidate`

Scope:

Read-only security review of authentication, authorization, API keys, identity,
proof/verification (including post-`5681d3c` TSA digest semantics), rate
limiting, audit integrity, error leakage, dependencies, and git-history secret
patterns.

Constraints honored:

- No code, test, config, or migration changes
- No live exploitation / abuse testing
- No secret values copied into this report (evidence is `file:path:line` only)
- Only deliverable file created under this step

---

## Executive Summary

Status: **PASS WITH FINDINGS**

No Critical findings in the reviewed scope.

Highest confirmed issues are **High** information-disclosure paths on legacy
HTTP handlers that return raw `Display` / SQL error strings to clients. The
normative `/v1` API error envelope does not show this pattern.

Core RC controls for account isolation on v1, hashed API keys / sessions,
identity historical verification after revoke, and TSA
`unavailable != proof invalid` behave as designed.

Release tag should not be created until High findings are remediated or
explicitly accepted as known limitations for pre-pilot / RC.

---

## Findings

### SEC-001

Severity: High  
Area: Error leakage  

Evidence:

- `src/api/verify.rs:15-28`
- `src/api/verify.rs:70-73`
- `src/api/verify.rs:80-83`
- `src/api/verify.rs:90-92`
- `src/api/verify.rs:115-118`
- `src/main.rs:124`

Description:

Legacy `/verify/*` uses a local `ApiError::Internal(String)` that serializes the
raw message into the JSON body. Handlers map failures with `e.to_string()`
(including `sqlx::Error`). The router is nested without API-key / session
middleware.

Risk:

Internal database / driver details may be exposed to unauthenticated callers who
can hit `/verify/{chain_id}` (and related legacy verify routes).

Recommendation:

Map all errors to a static internal message (same pattern as
`src/api/v1/errors.rs`). Prefer retiring or auth-gating legacy `/verify` in
favor of `/v1/verify` and `/public/verify`.

---

### SEC-002

Severity: High  
Area: Error leakage  

Evidence:

- `src/api/chains.rs:26-33`
- `src/api/account.rs:61-70`
- `src/api/account.rs:73-82`
- `src/api/account.rs:85-92`
- `src/main.rs:120-122`

Description:

Authenticated legacy handlers return `Result<…, String>` / `map_err(|e| e.to_string())`,
so Axum can surface database/serialization error text to API-key clients.

Risk:

SQL / internal error content disclosure to authenticated callers on
`/chains` and `/account/*`.

Recommendation:

Use a typed error type with opaque internal responses (v1 `ApiError::Internal`
pattern).

---

### SEC-003

Severity: Medium  
Area: Authorization  

Evidence:

- `src/service/ledger.rs:54-57`
- `src/api/events.rs:39-47`
- `src/api/v1/submit_event.rs:113-115`
- `src/main.rs:123`

Description:

Legacy `POST /events` maps `LedgerError::ChainAccessDenied` to HTTP **403** with
message indicating another account owns the chain. Normative v1 submit maps the
same condition to **404**.

Risk:

Account A can probe whether a guessed/leaked `chain_id` is owned by another
account (existence / ownership oracle). Not a direct data exfil of event bodies
via this path alone.

Recommendation:

Align legacy path with v1: foreign chain → 404 / not_found without ownership
wording.

---

### SEC-004

Severity: Medium  
Area: Authentication  

Evidence:

- `src/auth/password.rs:18-20`
- `src/auth/web_auth.rs:104`
- `src/auth/web_auth.rs:232`

Description:

Password policy accepts any password with length ≥ 8. No complexity or
breach-list checks. Storage uses Argon2id (positive control).

Risk:

Weak passwords remain valid despite strong hashing.

Recommendation:

Strengthen policy (length floor, blocklists / strength scoring) before broader
user exposure.

---

### SEC-005

Severity: Medium  
Area: Authentication  

Evidence:

- `src/auth/web_auth.rs:156-157`
- `src/auth/web_auth.rs:162-169`

Description:

Unknown email / missing password hash returns `InvalidCredentials` without
running Argon2 verify. Existing accounts always execute verify. Status codes are
unified; timing differs.

Risk:

Account-existence timing oracle on login.

Recommendation:

Always run a dummy Argon2 verification on missing-user paths.

---

### SEC-006

Severity: Medium  
Area: Identity  

Evidence:

- `src/api/identity_keys.rs:169-213`
- `src/service/identity_challenge.rs:87-105`

Description:

Identity key registration creates the key row, then marks the challenge used.
These steps are not wrapped in one transaction. Concurrent reuse can leave a
key inserted when challenge mark loses the race (account-scoped, not
cross-account IDOR).

Risk:

Broken single-use challenge invariant; possible duplicate registrations under
race.

Recommendation:

Single transaction: validate challenge → insert key → mark used, or insert with
conditional challenge update.

---

### SEC-007

Severity: Medium  
Area: Rate limiting  

Evidence:

- `src/middleware/login_rate_limit.rs:26-27`
- `src/api/v1/errors.rs` (v1 `RateLimited` IntoResponse — no Retry-After on this login path)
- Contrast: `src/middleware/public_rate_limit.rs:183-185`

Description:

Login rate limit returns `ApiError::RateLimited` without the `Retry-After` /
`X-RateLimit-*` headers applied by the public rate-limit middleware.

Risk:

Clients cannot reliably back off; weaker operational abuse hygiene vs public
endpoints.

Recommendation:

Attach the same rate-limit headers on login 429 responses.

---

### SEC-008

Severity: Medium  
Area: Data integrity  

Evidence:

- `migrations/20260628192818_init.sql:8-18`
- `src/api/v1/proof_material.rs:197-207`
- `src/service/ledger.rs:356-358` (placeholder signature then persist)

Description:

Event immutability is an application convention. Schema allows UPDATE/DELETE;
no append-only triggers/RLS found. Cryptographic detection (Merkle + Ed25519 +
optional TSA) catches tampering **without** the server signing key.

Risk:

A privileged DB actor (or compromised DB credentials) can rewrite history if
they can also re-sign (or if consumers trust DB rows without verify).

Recommendation:

Document operator trust boundary; consider DB append-only controls / audit
triggers for production hardening.

Requires code decision (if enforcing at DB layer).

---

### SEC-009

Severity: Low  
Area: Session / CSRF  

Evidence:

- `src/api/dashboard.rs:25-33`
- `src/api/dashboard.rs:173-198`
- `src/auth/session_store.rs:104-109`
- Contrast UI CSRF: `src/web/dashboard.rs:190-240`

Description:

Cookie-authenticated dashboard JSON mutations rely on `SameSite=Lax` without the
HTMX Origin/Referer checks used by dashboard UI routes.

Risk:

Residual CSRF if cookie SameSite posture changes.

Recommendation:

Apply Origin/Referer (or CSRF token) checks to all cookie-authenticated
state-changing JSON routes.

---

### SEC-010

Severity: Low  
Area: API key lifecycle  

Evidence:

- `migrations/20260715120000_accounts_and_api_keys.sql:9-16`
- `src/auth/mod.rs:55-56`
- Contrast desktop TTL: `src/service/desktop_tokens.rs:9`

Description:

API keys have revoke but no expiry. Desktop tokens have TTL.

Risk:

Leaked keys remain usable until revoked.

Recommendation:

Optional TTL / rotation policy; surface last-used metadata.

---

### SEC-011

Severity: Low  
Area: API key revoke  

Evidence:

- `src/service/accounts.rs:421-467`

Description:

Last-active-key protection is check-then-update without a single serializing
transaction.

Risk:

Concurrent revokes can leave an account with zero active keys (availability /
lockout), not privilege escalation.

Recommendation:

Transactional revoke with row lock / conditional update.

---

### SEC-012

Severity: Low  
Area: Identity / documentation  

Evidence:

- `docs/IDENTITY_MODEL.md:12`
- `docs/IDENTITY_MODEL.md:242-243`
- `src/bin/verify.rs` (no identity verification path)

Description:

IDENTITY_MODEL claims offline verifier resolves historical identity keys.
`evident-verify` checks server signature / merkle / structure only.

Risk:

Operators may over-trust offline identity claims. Not a server IDOR.

Recommendation:

Align docs with CLI capabilities, or implement offline identity verify.

Deferred documentation item (related to Stage 13.7 doc audit OPEN items).

---

### SEC-013

Severity: Informational  
Area: Dependencies  

Evidence:

- `Cargo.toml` (caret ranges for axum/sqlx/crypto crates)
- `Cargo.lock` present (transitive pins at lock time)
- Environment: `cargo audit` command not installed (`cargo audit --version` → no such command)

Description:

**Dependency vulnerability scanning was not performed.**  
`cargo audit` was unavailable in the review environment. This must not be read
as “dependencies were checked and found clean.” Crypto crates observed in
manifests/lockfile (`ed25519-dalek`, `sha2`, `hmac`, `argon2`, `rustls`,
transitive `openssl` / `ring`) are listed for inventory only.

Risk:

Unknown CVEs until an actual `cargo audit` (or equivalent) run is recorded.

Recommendation:

Run `cargo audit` in CI or a network-enabled environment before RC tag; record
the real result in the security decision log. Do not update dependencies in
this review step.

---

### SEC-014

Severity: Informational  
Area: Authentication  

Evidence:

- `src/auth/session_store.rs:14`
- `src/auth/api_key.rs:28`
- Contrast: `src/auth/password.rs:33` (`OsRng`)

Description:

Session / API-key / desktop secret generation uses `thread_rng` rather than
explicit `OsRng`.

Risk:

Low if ThreadRng remains OS-seeded CSPRNG; consistency / clarity concern.

Recommendation:

Prefer `OsRng` / `getrandom` for long-lived secrets.

---

## Areas with no confirmed issues (reviewed scope)

### Authentication / sessions

No authentication Critical/High storage or session-fixation issues identified in
reviewed scope.

Positive controls:

- Session token: 32 random bytes; store SHA-256 only — `src/auth/session_store.rs:12-19`, `40-60`
- TTL 30 days absolute; expiry enforced on lookup — `src/auth/session_store.rs:10`, `22-26`, `73-88`
- Login regenerates session; prior sessions deleted — `src/auth/session_store.rs:41-42`
- Cookie: HttpOnly + SameSite=Lax (+ Secure when not `dev_mode`) — `src/auth/session_store.rs:104-109`, `src/auth/web_auth.rs:177`
- Logout deletes session by hash — `src/auth/web_auth.rs:198-220`
- Invalid/expired session → Unauthorized — `src/middleware/session_auth.rs:43-45`

Auth tests present: `tests/web_auth.rs`, `tests/desktop_auth.rs`,
`tests/accounts_api.rs`, `tests/dashboard_ui.rs` (session/CSRF/UI). Gaps: cookie
flag assertions, dummy-hash login timing, JSON CSRF Origin tests, concurrent
last-key revoke.

### Authorization

No confirmed path for Account A to read Account B private event/proof/verify/
backup/identity-key data via ownership-checked v1 / dashboard / backup APIs.

Evidence:

- `src/api/v1/event_access.rs:37-79`
- `src/api/v1/proof.rs:29`
- `src/api/v1/verify.rs:43`
- `src/api/v1/identity_key_revoke.rs:62-64`
- `src/service/identity_dashboard.rs:73`, `104-108`
- `src/service/backup.rs:167`, `188`
- `src/auth/mod.rs:51-68`

Remaining authorization finding: SEC-003 (legacy ownership oracle), not v1
data exfil.

### API keys

No plaintext API-key storage identified.

Positive controls:

- Hash of secret only — `src/auth/api_key.rs:42-46`, `src/service/accounts.rs:505-511`
- Auth requires non-revoked — `src/auth/mod.rs:44-68`
- List returns prefix only — `src/auth/api_key.rs:60-74`
- Full key returned once at creation — `src/api/accounts.rs:224-233`

### Identity

No issue found on revoked-key historical verification for server `/v1/verify`:

- New signatures reject revoked keys — `src/service/identity_signing.rs:54-56`
- Historical verify does not require non-revoked — `src/service/identity_verification.rs:34-35`, `64-69`
- Revoke ownership-checked — `src/api/v1/identity_key_revoke.rs:62-73`

### Verification / TSA

Confirmed: `unavailable != invalid` / does not alone set proof failure.

Evidence:

- `is_failure()` only `Failed` — `src/tsa/types.rs:55-57`
- Missing CA → `Unavailable` — `src/tsa/read_verify.rs:111-118`
- Unavailable does not fail envelope — `src/api/v1/proof_state.rs:62-66`
- Failure signal only for `TsaStatus::Failed` — `src/api/v1/proof_state.rs:95-99`
- Post-`5681d3c` OpenSSL `-digest` — `vendor/notary-tsa/src/openssl_provider.rs:145-159`, `261-303`

`pending` is a `proof_status` gate, not a TSA `verification_status` value.

### Rate limiting

Public verify / certificate / register: IP-keyed buckets, Retry-After, proxy
header gate, and 429 disclosure hygiene reviewed positively.

Evidence:

- `src/state/rate_limiter.rs:15-59`, `110-131`
- `src/middleware/public_rate_limit.rs:121-206`
- Tests: `tests/v1_public_rate_limit.rs`

No live abuse testing performed.

### Audit integrity

No production HTTP DELETE-event path found. Integrity model is parent links +
Merkle + Ed25519 (+ optional TSA), not a classic `prev_hash` chain field.

Evidence:

- `src/merkle.rs:25-37`
- `src/service/verification.rs:21-42` (structure), `53+`
- Signing message format — `src/signing.rs:55-56`, `96-99`

Proof substitution without server signing key fails verification. DB-level
append-only is SEC-008.

### Dependencies

**Not performed.** `cargo audit` was unavailable (`cargo audit --version` →
command missing). This is not a clean/pass result. Lockfile present; no
dependency upgrades performed.

### Git history scan

Command pattern search over `git log -p --all` for
`api_key` / `secret` / `password` / `BEGIN.*PRIVATE` style markers was executed.

Result:

No obvious secret patterns found in git history scan.

Observed hits were documentation headers, test fixture names, and identifier
names (e.g. module/header `X-API-KEY`). No PEM private-key blocks identified.
Secret values were not copied into this report.

---

## Verified Controls

| Control | Result |
|---------|--------|
| Authentication | Reviewed — hashed sessions, TTL, logout; Medium password/timing findings |
| Authorization | Reviewed — v1 isolation OK; Medium legacy ownership oracle |
| API keys | Reviewed — hashed storage / prefix list / revoke OK; Low TTL/race |
| Identity | Reviewed — historical verify after revoke OK; Medium register race |
| Verification | Reviewed — Merkle/signature/TSA; `unavailable != invalid` confirmed |
| Rate limiting | Reviewed — public OK; login Retry-After gap |
| Audit integrity | Reviewed — crypto model OK; DB append-only not enforced |
| Dependencies | **Not performed** (`cargo audit` unavailable) — Informational |
| Git history scan | No obvious secret patterns found |

---

## Open Questions

1. Are legacy `/verify`, `/chains`, `/events`, `/account` routes still in the
   supported production surface for RC, or CLI/compat-only?
2. Should High error-leakage findings block RC tag, or be accepted with a
   Known Limitation until routes are retired?
3. Is operator/DB compromise in the RC threat model (SEC-008)?
4. When will `cargo audit` be required in CI?

---

## Release Recommendation

**Do not create a release tag in this step.**

Recommended path:

1. Remediate **SEC-001** and **SEC-002** (or formally accept + document as
   Known Limitations and disable/unauth-gate legacy verify if unused).
2. Schedule Medium items (SEC-003–SEC-008) for Stage 13.7.2 follow-up /
   hardening.
3. Re-run or spot-check after remediations; then decide RC acceptance /
   release tag in a separate step.

Overall for current baseline `75cfc12`: **PASS WITH FINDINGS** — suitable to
continue RC checklist work, not yet “security clear for tag” without High
decision.
