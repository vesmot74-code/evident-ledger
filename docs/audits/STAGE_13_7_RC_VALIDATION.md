# Stage 13.7 RC Validation

Date: 2026-08-01

Branch: `stage-13.7-release-candidate` @ `edcb364`

Merge: `edcb364` — `merge: Stage 13.7 TSA trust hardening`

Scope: post-merge audit / validation only. No Rust changes. No fix commits.
This file left uncommitted for manual review.

---

## Git history

- [x] **PASS**

Details:

`git log --oneline --decorate -20` shows required lineage under `edcb364`.

Ancestor checks (`git merge-base --is-ancestor <commit> HEAD`):

| Commit | Message | Ancestor of HEAD |
|---|---|---|
| `87a320f` | fix(tsa): validate trust material configuration | yes |
| `00eb40b` | docs: update TSA PR description with full verification reasons | yes |
| `5bbcebc` | docs: record manual TSA startup validation (Stage 13.7) | yes |
| `ef531c0` | docs: add Stage 13.7 final review for TSA trust hardening PR | yes |

---

## TSA code validation

- [x] **PASS**

Details (`git diff 6ffd376..HEAD -- src/tsa src/main.rs`):

| Expectation | Result |
|---|---|
| Startup trust validation in `main.rs` | Present (`check_tsa_configuration` + `enforce_tsa_trust_at_startup`) |
| `trust_config` module | Present (`src/tsa/trust_config.rs`, 174 lines added) |
| `verification_reason` metadata | Present (`TsaVerificationReason` + optional JSON field in `proof_state.rs`) |
| Existing TSA flow retained | `parse_and_validate_tsr` + `verify_tsr_bytes` still called on read path |
| No new `TsaVerificationStatus` variants | Same four: `Verified`, `VerifiedCached`, `Failed`, `Unavailable` (matches `6ffd376`) |

`TsaVerificationReason` variants:

- `TrustMaterialMissing`
- `TrustMaterialInvalid`
- `TsaNetworkUnavailable`
- `VerificationFailed`

Wire serialization (`as_str`):

- `trust_material_missing`
- `trust_material_invalid`
- `tsa_network_unavailable`
- `verification_failed`

---

## TSA startup validation document

- [x] **PASS**

Source: `docs/audits/STAGE_13_7_STARTUP_VALIDATION.md`

| Mode | Recorded evidence | Result |
|---|---|---|
| Production (`APP_ENV=production`, FREETSA_* unset) | Message `TSA trust configuration invalid: missing CA certificate path`; exit code `1` | PASS |
| Non-production (development) | `WARN` + process continues (`Evident Ledger running on http://0.0.0.0:3000`) | PASS |
| Write-path stamping section | Explicitly does **not** claim read-path crypto verification | PASS (no overclaim) |

---

## Automated tests

- `cargo test --lib tsa`:

  result: **ok** — `24 passed; 0 failed`

- `cargo test --test tsa_read_verify`:

  result: **ok** — `4 passed; 0 failed`

---

## Documentation review

### README

STATUS: **FINDING (non-blocking)**

README mentions RFC 3161 trusted timestamping support at a high level. It does
not document:

- production fail-closed trust-material startup policy;
- `verification_reason` metadata;
- distinction between trust-material problems vs TSA availability problems.

No README change made (per validation constraints).

### SECURITY

STATUS: **FINDING (non-blocking)**

`SECURITY.md` § Timestamp authority describes read-path verification / cache /
stub gates, but does **not** yet document:

- production: invalid trust material → startup `exit(1)`;
- non-production: warning only and continue;
- additive `verification_reason` vocabulary.

No SECURITY.md change made.

### CHANGELOG

STATUS: **FINDING (non-blocking)** — *Release notes improvement recommended*

`CHANGELOG.md` still ends at `[1.0.0] - 2026-07-08` and has no Stage 13.7 entry
for:

- TSA trust hardening;
- production fail-closed policy;
- `verification_reason` metadata.

Not a release blocker per this validation brief (separate release notes /
audit docs already cover Stage 13.7).

---

## Findings

1. **Release notes improvement recommended** — add Stage 13.7 TSA trust
   hardening / fail-closed / `verification_reason` to `CHANGELOG.md` (or
   equivalent release notes) before final public tag packaging.
2. **SECURITY.md lag** — TSA section should eventually describe startup trust
   policy (prod fail-closed / non-prod warn) and optional
   `verification_reason`.
3. **README lag** — high-level TSA blurb does not distinguish trust-material
   misconfiguration vs TSA availability.
4. **Future enhancement:** emit `tsa_network_unavailable` on TSA network /
   outage paths — enum/reserved value exists; no runtime constructor today
   (acceptable for this merge).

None of the above are RC merge blockers for the TSA trust-hardening change set.

---

## Final status

**READY FOR RELEASE**

(with non-blocking documentation follow-ups listed under Findings)
