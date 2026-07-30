# Stage 13.7 Documentation Audit

Date: 2026-07-31  
Branch: `stage-13.7-release-candidate`  
Baseline:

```text
43f05f4 docs: record Stage 13.7 TSA verification fix
5681d3c fix(tsa): align RFC3161 verification digest semantics
2188194 docs: clarify Stage 13.6 known test limitation
bc42d3e docs: finalize Stage 13.6 production readiness audit
6695770 fix: restore landing page download and auth navigation contract
```

Working tree at audit start: clean.

**Constraint (audit creation):** findings only; code/tests/API behavior not changed.

**P0 documentation alignment (Stage 13.7.1 docs):** in progress against this report — see Finding 1–3 statuses below.

---

## Scope

Reviewed files:

- `README.md`
- `SECURITY.md`
- `docs/API.md`
- `docs/VERIFY_MODEL.md`
- `docs/IDENTITY_MODEL.md`
- `docs/BILLING_MODEL.md`
- `docs/audits/STAGE_13_7_RELEASE_CANDIDATE.md`
- `docs/audits/STAGE_13_7_TSA_INVESTIGATION.md`

Code cross-checks (read-only): `src/api/v1/verify.rs`, `src/api/v1/proof_state.rs`, `src/api/v1/account.rs`, `src/service/capabilities.rs`, `src/service/ledger.rs`, `src/tsa/read_verify.rs`, `vendor/notary-tsa/src/openssl_provider.rs`, identity migration, ADR TSA read-path (referenced from SECURITY).

---

## Findings

### Finding 1 — API.md `/v1/verify` `file{}` shape is wrong

Status: **RESOLVED**  
Severity: high  
Resolution commit: pending documentation alignment commit  

Description (original):

`docs/API.md` §7 documented `file.status` (`NOT_PERFORMED` / `VALID` / `TAMPERED`) instead of the implemented `provided` / `provided_hash` / `is_valid_file_hash` fields.

Resolution:

`docs/API.md` §7 updated to match `src/api/v1/verify.rs` / `file_verification.rs`. Aligned with VERIFY_MODEL Layer 3.

---

### Finding 2 — API.md claims `/v1/verify` returns `tsa`

Status: **RESOLVED**  
Severity: high  
Resolution commit: pending documentation alignment commit  

Description (original):

`docs/API.md` §7 claimed `/v1/verify` returns a `tsa` object like proof.

Resolution:

Documented factual contract: no `tsa` field on `/v1/verify`; TSA gates via `proof_status`; token detail remains on `GET /v1/proof` (`verification_status`: `verified` | `verified_cached` | `failed` | `unavailable`).

---

### Finding 3 — VERIFY_MODEL lacks digest-imprint write/read alignment after `5681d3c`

Status: **RESOLVED**  
Severity: high  
Resolution commit: pending documentation alignment commit  

Description (original):

VERIFY_MODEL lacked digest write → RFC3161 imprint → `openssl ts -verify -digest` flow after `5681d3c`.

Resolution:

Added “RFC3161 TSA verification (digest imprint)” under Layer 1, including status vocabulary, API exposure rules, unavailable ≠ proof failed, and historical note for pre-`5681d3c` `-data` behavior.

---

### Finding 4 — SECURITY.md incomplete on TSA verify outcomes / unavailable semantics

Status: **OPEN** — Deferred to Stage 13.7.2 / future follow-up  
Severity: medium  
Description:

SECURITY.md § Timestamp authority correctly describes read-path re-validation and cache limitation, and does **not** claim HSM/KMS/automatic rotation (good).

Gaps:

1. Does not explicitly state that `verification_status=unavailable` ≠ invalid proof / ≠ `proof_status=failed` (code: only `Failed` sets TSA failure signal; missing CA → `unavailable`).
2. Does not list full wire vocab: `verified` | `verified_cached` | `failed` | `unavailable` (keep `pending` as `proof_status`, not TSA verify status).
3. Linked ADR (`docs/design/ADR_TSA_READ_PATH_VERIFICATION.md`) still says OpenSSL `ts -verify` without `-digest`; post-`5681d3c` production uses `-digest`. ADR is outside this audit’s edit scope but is the SECURITY pointer target.

Recommended action:

Clarify unavailable ≠ proof invalid; list `verification_status` enum; separately update ADR to `-digest` when docs fixes are approved.

---

### Finding 5 — API.md lists `GET /v1/account/capabilities` as live contract

Status: **OPEN** — Deferred to Stage 13.7.2 / future follow-up  
Severity: medium  
Description:

API.md §3/§8 presents `GET /v1/account/capabilities` as a normative endpoint. Implementation (`src/api/v1/account.rs`) returns `NotImplemented`. Live capabilities are served from `GET /account/capabilities` (non-`/v1` surface).

Recommended action:

Mark `/v1/account/capabilities` as not implemented / placeholder, and point to `/account/capabilities` as the current surface.

---

### Finding 6 — BILLING_MODEL understates paid write blocker for qualified TSA

Status: **OPEN** — Deferred to Stage 13.7.2 / future follow-up  
Severity: medium  
Description:

BILLING_MODEL §5 access matrix shows paid + `active` → `/v1` writes allowed within limits. Code (`capabilities.tsa_available()` only true for `TsaMode::Machine`; paid plans seed `qualified`) causes `QualifiedTsaUnavailable` on ledger submit → mapped to API internal error for v1 writes.

Subscription/billing state can be healthy while event writes still fail until qualified TSA is available or plan semantics change.

Recommended action:

Add a footnote to BILLING_MODEL §5: paid entitlement is necessary but not sufficient for writes while plan `tsa_mode=qualified` and only machine TSA is wired.

---

### Finding 7 — IDENTITY_MODEL `verified_at` schema mismatch

Status: **OPEN** — Deferred to Stage 13.7.2 / future follow-up  
Severity: medium  
Description:

IDENTITY_MODEL §2 shows `verified_at TIMESTAMPTZ NULL` and allows NULL for admin/migration flows. Migration `migrations/20260718170000_create_identity_keys.sql` defines `verified_at TIMESTAMPTZ NOT NULL DEFAULT now()`.

Generate / register / sign / revoke / historical verification flows are otherwise consistent; no invented HSM/escrow features found.

Recommended action:

Align schema text to `NOT NULL DEFAULT now()`; remove or rewrite NULL-admin wording as unsupported in current schema.

---

### Finding 8 — API.md omits `identity_signature` on submit/verify envelopes

Status: **OPEN** — Deferred to Stage 13.7.2 / future follow-up  
Severity: medium  
Description:

Submit accepts optional identity material; verify returns `identity_signature` (`present` / `valid` / `reason` / `fingerprint` / `key_id`) when present (`src/api/v1/verify.rs`). IDENTITY_MODEL covers the feature; API.md verify/submit contracts do not fully document the wire shape.

Note (P0 partial): `/v1/verify` examples now include `"identity_signature": null` to match the response builder; full identity envelope documentation remains deferred.

Recommended action:

Document optional identity fields on `POST /v1/events` and `GET /v1/verify` in API.md, consistent with IDENTITY_MODEL wire shapes.

---

### Finding 9 — Stage 13.7 investigation body still reads as “current” pre-fix state

Status: stale  
Severity: low  
Description:

`STAGE_13_7_RELEASE_CANDIDATE.md` correctly has Discovery (historical) + Resolved; RC status is “TSA blocker resolved, pending final release checklist” (not “RC approved”). Good.

`STAGE_13_7_TSA_INVESTIGATION.md` has a correct `## Resolution` for `5681d3c`, but earlier sections still say production uses `-data`, “fix not implemented”, and header “diagnosis complete — no code/test changes”. A reader skimming the middle can treat pre-fix diagnosis as current.

Recommended action:

Prefix investigation body as historical snapshot as of diagnosis; leave Resolution as current. Optionally add “superseded by Resolution” near Q3/proposed fix.

---

### Finding 10 — README TSA coverage is thin but not false

Status: ok note  
Severity: info  
Description:

README mentions RFC 3161-compatible timestamp verification support and private/public verify pipelines. No outdated OpenSSL `-data` wording. No unconfirmed claims of `production ready` / `fully production ready` / `enterprise ready` / bare `guaranteed`.

Recommended action:

Optional one-liner pointing to VERIFY_MODEL / ADR for digest-imprint semantics. No mandatory fix for overclaim risk.

---

## Verification

| Area | Result |
|------|--------|
| README | PASS |
| SECURITY | FINDINGS (OPEN — deferred) |
| API | P0 RESOLVED (`file{}`, verify TSA absence); P1 OPEN (capabilities, identity envelope detail) |
| VERIFY | P0 RESOLVED (digest imprint flow) |
| IDENTITY | FINDINGS (OPEN — deferred) |
| BILLING | FINDINGS (OPEN — deferred) |
| STAGE_13_7 RC / investigation | FINDINGS (stale narrative; resolution sections OK) |

---

## Final status

P0 documentation alignment (this stage): **YES** (pending commit)

Full documentation synchronized: **NO** (P1/P2 remain OPEN)

Changes required beyond P0: **YES**

| Priority | Items | Status |
|----------|-------|--------|
| P0 | #1 API file verification, #2 API verify TSA, #3 VERIFY_MODEL TSA flow | **RESOLVED** (pending documentation alignment commit) |
| P1/P2 | #4 SECURITY/ADR, #5 capabilities, #6 billing, #7 verified_at, #8 identity_signature, #9 investigation tense | **OPEN** — Deferred to Stage 13.7.2 / future follow-up |

**Stop before commit.** Await confirmation to run:

```bash
git add docs/API.md docs/VERIFY_MODEL.md docs/audits/STAGE_13_7_DOC_AUDIT.md
git commit -m "docs: align API.md and VERIFY_MODEL.md with TSA digest fix (P0)"
```
