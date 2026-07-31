# Stage 13.7 — Final Review (PR #3)

Date: 2026-08-01

Reviewed branch: `stage-13.7-pilot-validation` @ `5bbcebc`

Base: `stage-13.7-release-candidate` (merge-base / compare point `6ffd376`)

PR: https://github.com/vesmot74-code/evident-ledger/pull/3

Scope of this review: independent pre-merge audit of code, documentation, and
PR description against the repository. **No Rust code was modified** for this
review. Deliverable: this document only.

Commits in scope:

| Commit | Message |
|---|---|
| `87a320f` | `fix(tsa): validate trust material configuration` |
| `00eb40b` | `docs: update TSA PR description with full verification reasons` |
| `5bbcebc` | `docs: record manual TSA startup validation (Stage 13.7)` |

---

## Verdict

**PASS WITH NOTES — merge-ready for Stage 13.7 TSA trust hardening.**

Core claims in PR #3 match the implementation. No new `TsaVerificationStatus`
wire values were introduced. RFC3161 digest verification path was not changed
by this PR. Remaining notes are documentation / checklist hygiene, not blockers.

---

## 1. PR description vs code

### Claims checked

| PR claim | Evidence | Result |
|---|---|---|
| Pure `check_tsa_configuration(ca, untrusted)` without env reads | `src/tsa/trust_config.rs` — function takes `Option<&Path>` only | PASS |
| Env parsing in callers (`FREETSA_*`) | `freetsa_trust_path_options_from_env()` + `main.rs` | PASS |
| Production refuses start → `exit(1)` | `enforce_tsa_trust_at_startup(true, …)` | PASS |
| Non-production warns and continues | `tracing::warn!(…)` branch | PASS |
| Existing `verification_status` kept (incl. `unavailable`) | `TsaVerificationStatus` variants unchanged vs `6ffd376` | PASS |
| RFC3161 remains `-digest` | No `vendor/notary-tsa` changes in `6ffd376..HEAD`; `verify_tsr_bytes` still documents `-digest` | PASS |
| Four additive `verification_reason` values | `TsaVerificationReason::as_str()` returns all four strings | PASS |
| No new public status enum values | Only `TsaVerificationReason` + `TsaVerificationOutcome` added | PASS |
| Optional reason on proof JSON; DB cache status unchanged | `proof_state.rs` inserts reason only when present; `cache_value()` still `verified`/`failed`/`unavailable` | PASS |

### Diff coverage (`87a320f` + docs)

Code/docs touched by the PR (vs `6ffd376`):

- `src/tsa/trust_config.rs` (new)
- `src/tsa/types.rs`, `read_verify.rs`, `lib.rs`
- `src/main.rs`, `src/api/v1/proof_state.rs`
- `tests/tsa_read_verify.rs` (`.status` accessors only)
- `docs/audits/STAGE_13_7_TSA_INVESTIGATION.md`
- `docs/audits/STAGE_13_7_RELEASE_CANDIDATE.md`
- `docs/audits/STAGE_13_7_STARTUP_VALIDATION.md` (`5bbcebc`)

No unstated code areas (billing, identity, Merkle, migrations) appear in the
diff.

### Note — stale PR checklist item

PR Test plan still shows:

```text
- [ ] Runtime startup behavior manually verified: …
```

Repository already contains `docs/audits/STAGE_13_7_STARTUP_VALIDATION.md`
(`5bbcebc`) recording PASS for both production fail-closed and non-production
warn-and-continue. **Recommendation (non-blocking):** mark that checklist item
`[x]` in the GitHub PR body before merge. No code change required.

---

## 2. `TsaVerificationStatus`

Command basis: `git diff 6ffd376..HEAD -- src/tsa/types.rs`

### Status vocabulary

Before and after, public status variants remain:

- `Verified` → `verified`
- `VerifiedCached` → `verified_cached`
- `Failed` → `failed`
- `Unavailable` → `unavailable`

`as_str()` / `cache_value()` mappings for these variants are unchanged.

### What was added (not a status)

- `TsaVerificationReason` — additive diagnostic enum
- `TsaVerificationOutcome { status, reason }` — internal/API helper wrapping
  status + optional reason

**Finding:** PR statement *“No new public status enum values were introduced”*
is **correct**.

---

## 3. `verification_reason` completeness

Wire strings from `TsaVerificationReason::as_str()`:

| Enum variant | Wire value | Runtime construction today |
|---|---|---|
| `TrustMaterialMissing` | `trust_material_missing` | Yes — missing path(s) |
| `TrustMaterialInvalid` | `trust_material_invalid` | Yes — path set but not a file |
| `VerificationFailed` | `verification_failed` | Yes — imprint / OpenSSL / stub failures |
| `TsaNetworkUnavailable` | `tsa_network_unavailable` | **No** — reserved only |

PR description correctly lists all four values present in code.

**Note (non-blocking):** `tsa_network_unavailable` is reserved vocabulary; no
call site currently returns it. Investigation / RC docs already describe it as
reserved. This is not a PR false claim — the value exists — but operators should
not expect it on live responses until a future path maps network/OpenSSL spawn
failures explicitly.

---

## 4. Startup policy

`src/main.rs` after `AppConfig::from_env()`:

1. Load path options from env (no file I/O in parser).
2. `check_tsa_configuration`.
3. On error: `is_production_tsa_env(ENVIRONMENT, APP_ENV)` → fatal `exit(1)` or
   `tracing::warn!`.

Manual evidence: `docs/audits/STAGE_13_7_STARTUP_VALIDATION.md` — PASS for both
modes (missing CA path).

Production detection uses runtime env labels only (no `cfg!(feature = …)`).

---

## 5. Crypto / RFC3161 path

- This PR does **not** modify `vendor/notary-tsa`.
- Read-path still calls `notary_tsa::verify_tsr_bytes` after trust config OK.
- Production digest verify remains `openssl ts -verify -digest` (prior fix
  `5681d3c`).
- Smoke/query helpers may still use `-data` for file-hash flows; that is
  outside this PR’s trust-config scope and was not reintroduced for imprint
  verification.

**Finding:** PR claim that digest verification is unchanged is **correct**.

---

## 6. Compatibility

| Surface | Assessment |
|---|---|
| API `tsa.verification_status` | Unchanged vocabulary |
| API `tsa.verification_reason` | New **optional** field when reason present |
| DB `tsa_tokens.verification_status` | Still `verified` / `failed` / `unavailable` only; no migration |
| `unavailable ≠ proof failed` | Preserved (`is_failure()` only on `Failed`) |
| Clients ignoring unknown keys | Safe for additive reason |

### Behavioral tightening (documented, intentional)

Previous vendor helper `freetsa_trust_paths()` defaulted missing
`FREETSA_UNTRUSTED_CERT_PATH` to `tsa.crt`. New caller path requires both env
vars to be set (empty → missing). Read-path and startup now treat unset
untrusted path as `trust_material_missing` rather than silently probing cwd
`tsa.crt`. This is a deployment-config hardening, not a status-enum break.

---

## 7. Documentation alignment

| Document | Alignment |
|---|---|
| PR #3 body | Matches code; reason list complete (4/4) |
| `STAGE_13_7_TSA_INVESTIGATION.md` follow-up | Matches reason table + `-digest` confirmation |
| `STAGE_13_7_RELEASE_CANDIDATE.md` hardening section | Full four reasons + “no new status values” |
| `STAGE_13_7_STARTUP_VALIDATION.md` | Manual startup PASS recorded |

No contradictory claim found that this PR reintroduces `-data` for digest
imprint verify, or adds a `Misconfigured` status.

---

## 8. Tests (observed scope)

Present in tree for this change:

- Unit tests in `trust_config.rs` (missing CA / missing TSA cert / valid /
  not-a-file / production env detection) — **no** `std::env::set_var` in that
  module’s tests.
- Existing `read_verify` / `tsa_read_verify` updated to use
  `TsaVerificationOutcome.status`.

This review did not re-execute the full suite; prior pilot work recorded
relevant unit/integration passes for trust/read-verify paths.

---

## Findings summary

| ID | Severity | Finding | Action |
|---|---|---|---|
| FR-1 | Info | PR checklist still unchecked for manual startup despite `STAGE_13_7_STARTUP_VALIDATION.md` | Update PR body checkbox only |
| FR-2 | Info | `tsa_network_unavailable` is reserved (defined, not emitted) | Accept; keep docs wording “reserved” |
| FR-3 | Info | Untrusted path no longer defaults to `tsa.crt` | Accept as intentional hardening |

**Blockers:** none.

---

## Merge recommendation

Approve merge of PR #3 into `stage-13.7-release-candidate` for Stage 13.7 TSA
trust-material hardening, after optionally ticking the manual-startup checklist
item in the PR description (FR-1).

Suggested post-merge operator note: production deployments must set both
`FREETSA_CA_CERT_PATH` and `FREETSA_UNTRUSTED_CERT_PATH` to real certificate
files before process start.
