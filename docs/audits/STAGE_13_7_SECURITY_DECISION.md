# Stage 13.7.2-A — Security Decision Log

Date: 2026-07-31

Reviewed by: Human decisions recorded (Stage 13.7.2-A finalization)

Source review: `docs/audits/STAGE_13_7_SECURITY_REVIEW.md`

Baseline: `stage-13.7-release-candidate` @ `75cfc12`

---

## High findings

| Finding | Decision | Rationale | Action | Owner | Target |
|---|---|---|---|---|---|
| SEC-001 | **Fix before RC** | Legacy `/verify/*` exposes internal error details through raw error strings (`e.to_string()` / SQL driver text); route is unauthenticated. Direct information disclosure. | Introduce stable public error envelope; keep internal details only in server logs | TBD | Stage 13.7.2-B |
| SEC-002 | **Fix before RC** | Same information disclosure class on authenticated `/chains` and `/account/*` (`Result<…, String>` / `e.to_string()`). | Unify legacy routes with opaque public error mapping; align with v1 error handling model | TBD | Stage 13.7.2-B |

---

## Medium findings — triage

| Finding | Decision | Rationale |
|---|---|---|
| SEC-003 | **Fix before RC** | Legacy `POST /events` exposes ownership via **403** + ownership wording for foreign `chain_id`; v1 maps same case to **404**. Align legacy with v1: `foreign object == not found`. Target: Stage 13.7.2-B. |
| SEC-004 | **Partial fix before RC** | Argon2id storage is correct; increase minimum password length to **12** characters before RC. Deferred (backlog): password breach blacklist / compromised-password database. |
| SEC-005 | **Accept for pilot** | Auth status codes already unified; remote timing exploitation is hard; pilot threat model does not justify blocking RC. Deferred: constant-time login hardening / dummy hash verification for unknown users. |
| SEC-006 | **Accept for pilot** | Confirmed: `identity_keys.fingerprint` has DB `UNIQUE` — duplicate fingerprint registration blocked. Remaining risk is challenge consumption atomicity (`used_at` is application-level). Deferred: atomic challenge consume + identity key insert transaction. |
| SEC-007 | **Fix before RC** | Low-cost consistency: add login rate-limit headers (`Retry-After` + rate-limit metadata) similar to public API throttling. Target: Stage 13.7.2-B. |
| SEC-008 | **Accept for pilot** | Tampering is detectable cryptographically; schema does not prevent UPDATE/DELETE. DB admin/operator compromise is outside current pilot threat model. Follow-up docs: update `SECURITY.md` / `THREAT_MODEL.md` in a later documentation hardening stage. |

---

## Final security decision table

| ID | Area | Decision | RC gate |
|---|---|---|---|
| SEC-001 | Error leakage (`/verify`) | Fix before RC | Blocks RC until fixed |
| SEC-002 | Error leakage (`/chains`, `/account`) | Fix before RC | Blocks RC until fixed |
| SEC-003 | Authz oracle (legacy `/events`) | Fix before RC | Blocks RC until fixed |
| SEC-004 | Password min length → 12 | Partial fix before RC | Blocks RC until partial fix done |
| SEC-005 | Login timing | Accept for pilot | Accepted risk |
| SEC-006 | Identity challenge race | Accept for pilot | Accepted risk |
| SEC-007 | Login Retry-After | Fix before RC | Blocks RC until fixed |
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
- [x] SEC-004 partial fix completed *(required — min length 12; Stage 13.7.2-B)*
- [x] SEC-007 fixed *(required — Stage 13.7.2-B)*
- [ ] cargo audit executed and result recorded *(recommended before tag; currently NOT PERFORMED)*

Accepted pilot risks (do not block RC once blockers above are closed):

- SEC-005
- SEC-006
- SEC-008

---

## Stage 13.7.2-B worklist (implementation — not this step)

1. SEC-001 / SEC-002 — opaque error envelopes for legacy `/verify`, `/chains`, `/account`
2. SEC-003 — legacy `/events` foreign chain → not found (v1 semantics)
3. SEC-004 — password minimum length 12
4. SEC-007 — login `Retry-After` + rate-limit headers

Do **not** mix accepted-risk documentation commits with the 13.7.2-B fix commit unless explicitly requested.

---

## Commit (after approval of this finalization)

```bash
git add docs/audits/STAGE_13_7_SECURITY_REVIEW.md \
        docs/audits/STAGE_13_7_SECURITY_DECISION.md
git commit -m "security: record Stage 13.7 review findings and RC decisions"
```

**Not executed in this step.**
