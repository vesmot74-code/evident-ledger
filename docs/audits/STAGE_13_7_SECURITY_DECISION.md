# Stage 13.7.2-A — Security Decision Log

Date: 2026-07-31

Reviewed by: Human decisions recorded (Stage 13.7.2-A finalization)

Source review: `docs/audits/STAGE_13_7_SECURITY_REVIEW.md`

Baseline: `stage-13.7-release-candidate` @ `75cfc12`

---

## High findings

| Finding | Decision | Rationale | Action | Owner | Target |
|---|---|---|---|---|---|
| SEC-001 | **RESOLVED** | Legacy `/verify/*` returns opaque `v1::ApiError::Internal` envelope; internal details stay in server logs only. | Introduce stable public error envelope; keep internal details only in server logs | Stage 13.7.2-B | Stage 13.7.2-B (`e4fa355`) |
| SEC-002 | **RESOLVED** | Legacy `/chains` and `/account/*` use the same opaque public error mapping as `/verify`. | Unify legacy routes with opaque public error mapping; align with v1 error handling model | Stage 13.7.2-B | Stage 13.7.2-B (`e4fa355`) |

---

## Medium findings — triage

| Finding | Decision | Rationale |
|---|---|---|
| SEC-003 | **RESOLVED** | Legacy `POST /events` no longer exposes ownership via **403** + ownership wording. Local `map_legacy_events_error` maps `ChainAccessDenied` and `ChainNotFound` to identical HTTP **404** `{"error":"not_found"}`. Shared `LedgerError::IntoResponse` unchanged. |
| SEC-004 | **RESOLVED** | Minimum password length increased to **12** characters. Policy applies to new password creation and password changes only. Existing password hashes are not invalidated retroactively. No forced password reset introduced for pilot stage. Deferred (backlog): password breach blacklist / compromised-password database. |
| SEC-005 | **Accept for pilot** | Auth status codes already unified; remote timing exploitation is hard; pilot threat model does not justify blocking RC. Deferred: constant-time login hardening / dummy hash verification for unknown users. |
| SEC-006 | **Accept for pilot** | Confirmed: `identity_keys.fingerprint` has DB `UNIQUE` — duplicate fingerprint registration blocked. Remaining risk is challenge consumption atomicity (`used_at` is application-level). Deferred: atomic challenge consume + identity key insert transaction. |
| SEC-007 | **RESOLVED** | Login rate-limit HTTP 429 now includes `Retry-After` from `FixedWindowLimiter` `decision.retry_after_secs`, matching public API throttling. Threshold and window unchanged. |
| SEC-008 | **Accept for pilot** | Tampering is detectable cryptographically; schema does not prevent UPDATE/DELETE. DB admin/operator compromise is outside current pilot threat model. Follow-up docs: update `SECURITY.md` / `THREAT_MODEL.md` in a later documentation hardening stage. |

### SEC-003 observation (implementation)

Legacy `POST /events` auto-claims unseen `chain_id` through existing
`INSERT … ON CONFLICT DO NOTHING` behavior in `ensure_chain_access_in_tx`.

Therefore chain existence observability through a successful first claim
(HTTP 200 on a fresh UUID) is a **separate** product/security design
consideration.

This remediation only closes ownership **error** disclosure:

```text
403 + ownership wording  →  generic 404 { "error": "not_found" }
```

for `LedgerError::ChainAccessDenied` and defensive `ChainNotFound` mapping on
the legacy `/events` handler only.

Out of scope for SEC-003: changing auto-claim / first-writer-wins behavior.

**Status:** RESOLVED (Stage 13.7.2-B — legacy `/events` mapper + regression tests).

---

## Final security decision table

| ID | Area | Decision | RC gate |
|---|---|---|---|
| SEC-001 | Error leakage (`/verify`) | **RESOLVED** | Closed |
| SEC-002 | Error leakage (`/chains`, `/account`) | **RESOLVED** | Closed |
| SEC-003 | Authz oracle (legacy `/events`) | **RESOLVED** | Closed |
| SEC-004 | Password min length → 12 | **RESOLVED** | Closed |
| SEC-005 | Login timing | Accept for pilot | Accepted risk |
| SEC-006 | Identity challenge race | Accept for pilot | Accepted risk |
| SEC-007 | Login Retry-After | **RESOLVED** | Closed |
| SEC-008 | DB append-only | Accept for pilot | Accepted risk |
| SEC-013 | Dependencies | Scan not performed | Recommend before tag |

---

## Dependency Audit

```
Dependency vulnerability scan:
cargo audit: NOT PERFORMED
Reason:
cargo-audit command unavailable in current environment.
This does not indicate a clean dependency state.
```

Recommend running `cargo audit` in CI or a network-enabled environment before
RC tag and recording the actual result here:

```
cargo audit result: [ ] not run yet   [ ] clean   [ ] findings (link/note)
Date run:
Environment:
```

---

## Accepted pilot risks

- SEC-005 — Login timing difference
- SEC-006 — Identity challenge single-use race
- SEC-008 — Database append-only enforcement (operator/DB compromise out of pilot threat model)

---

## RC blocking status

RC tag `v0.13.7-rc1` blocked until:

- [x] SEC-001 fixed *(required — Stage 13.7.2-B)*
- [x] SEC-002 fixed *(required — Stage 13.7.2-B)*
- [x] SEC-003 fixed *(required — Stage 13.7.2-B)*
- [x] SEC-004 fixed *(required — min length 12; Stage 13.7.2-B)*
- [x] SEC-007 fixed *(required — Stage 13.7.2-B)*
- [ ] cargo audit executed and result recorded *(recommended before tag; currently NOT PERFORMED)*

Accepted pilot risks (do not block RC once blockers above are closed):

- SEC-005
- SEC-006
- SEC-008

---

## Stage 13.7.2-B worklist

1. SEC-001 / SEC-002 — opaque error envelopes for legacy `/verify`, `/chains`, `/account` — **done** (`e4fa355`)
2. SEC-003 — legacy `/events` ownership error disclosure → generic 404 — **done** (`a1a626e`)
3. SEC-004 — password minimum length 12 — **done** (`69df13a`)
4. SEC-007 — login `Retry-After` on rate-limit 429 — **done** (`ed4cd62`)

### SEC-004 resolution notes

Minimum password length increased to 12 characters.
Policy applies to new password creation and password changes only.
Existing password hashes are not invalidated retroactively.
No forced password reset introduced for pilot stage.

**Commit:** `69df13ac4292125636c12b40343a5599ca4a333d`

### SEC-007 resolution notes

Login rate-limit HTTP 429 includes `Retry-After` derived from the shared
`FixedWindowLimiter` decision (`retry_after_secs`), consistent with public API
throttling. Rate-limit threshold and window were not changed.

**Commit:** `ed4cd629eff465f5b8215e788a3f0dd614801dc7`

---

## Commit (after approval of this finalization)

```bash
git add docs/audits/STAGE_13_7_SECURITY_REVIEW.md \
        docs/audits/STAGE_13_7_SECURITY_DECISION.md
git commit -m "security: record Stage 13.7 review findings and RC decisions"
```

**Not executed in this step.**

### cargo audit result (post-RC1 triage)

Executed: 2026-07-31 (after v0.13.7-rc1 was tagged)

Raw output:
`docs/audits/STAGE_13_7_CARGO_AUDIT.txt`

8 advisories found. Triaged against actual dependency graph and build output:

| Finding | Reachable (server) | Reachable (gui-app) | Action |
|---|---|---|---|
| lopdf (RUSTSEC-2026-0187) | No — used through PDF generation (`printpdf`); no untrusted PDF parsing path found | N/A | No action; re-check if PDF parsing is introduced |
| quick-xml (RUSTSEC-2026-0194, RUSTSEC-2026-0195) | No — not in server dependency graph | Yes — via `keyring` → zbus → zbus_xml on Linux secret-service backend | Local desktop surface only; track future upgrade |
| rustls-webpki 0.101.7 (RUSTSEC-2026-0098/0099/0104) | Yes — transitive via sqlx 0.7.4 → rustls 0.21.12 | N/A | Address through sqlx upgrade |
| sqlx 0.7.4 (RUSTSEC-2024-0363) | Yes — direct dependency | N/A | Upgrade to sqlx >=0.8.1 in separate migration stage |
| rsa 0.9.10 (RUSTSEC-2023-0071) | No — inactive sqlx-mysql dependency path; not present in release build | N/A | No action |
| paste, rustls-pemfile, ttf-parser | Maintenance warnings | Maintenance warnings | Backlog |
| spin | Yanked crate warning | N/A | Backlog |

Process note:

This scan was executed after `v0.13.7-rc1` was already tagged and pushed.
The tag is intentionally not rewritten.

Triage confirms no unresolved exploitable server-side RC blocker.
The remaining sqlx/rustls modernization is tracked separately before
the final non-RC release.
