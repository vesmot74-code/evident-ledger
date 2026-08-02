# Stage 14.1 Post-Merge Validation

Date: 2026-08-02

Status: **PASS**

Baseline commit: `107a3fb`

Related milestone: `milestone-stage-14-signing-key-governance`

Branch: `stage-14.1-post-merge-validation`

---

## Purpose

Record reproducibility evidence that Stage 14 (signing key governance) introduced
**documentation only** and did not alter runtime cryptographic or verification
contracts.

Stage 14 merge contents:

- `docs/design/ADR_SIGNING_KEY_GOVERNANCE.md`
- `docs/audits/STAGE_14_SIGNING_KEY_READINESS.md`

Confirmed after merge:

| Area | Result |
|------|--------|
| Runtime code | Unchanged (docs-only diff vs `62a39d5`) |
| `proof_v1` schema | Unchanged |
| API contract | Unchanged |
| TSA model | Unchanged |

---

## Validation checks

### 1. Git state

Command:

```bash
git status
```

Validation context:

The Stage 14 merge baseline (`107a3fb`) was checked before creation of this audit artifact.

Expected:

No modified tracked files after the Stage 14 merge.

Result:

PASS — tracked repository state was clean before adding this validation document.

Note:

`STAGE_14_1_POST_MERGE_VALIDATION.md` itself was intentionally untracked until committed.

### 2. Diff scope (Stage 14 merge)

Command:

```bash
git diff 62a39d5..107a3fb --stat
```

Observed:

```text
 docs/audits/STAGE_14_SIGNING_KEY_READINESS.md |  31 +++++++
 docs/design/ADR_SIGNING_KEY_GOVERNANCE.md     | 112 ++++++++++++++++++++++++++
 2 files changed, 143 insertions(+)
```

`git diff 62a39d5..107a3fb --name-only`:

```text
docs/audits/STAGE_14_SIGNING_KEY_READINESS.md
docs/design/ADR_SIGNING_KEY_GOVERNANCE.md
```

Expected: only those two documentation files.

Result: **PASS**

### 3. Proof schema contract

Command:

```bash
cargo test --test proof_schema_contract
```

Observed:

```text
running 3 tests
test future_proof_schema_changes_require_version_bump ... ok
test proof_v1_core_fields_unchanged ... ok
test tsa_fields_are_not_inside_proof_v1_object ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Expected: 3 passed.

Result: **PASS**

### 4. Identity verification regression

Command:

```bash
cargo test --test v1_verify_identity
```

Observed:

```text
running 7 tests
…
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Expected: 7 passed.

Result: **PASS**

---

## Security boundary confirmation

Stage 14.1 did not introduce:

- signing key rotation
- HSM/KMS integration
- new key identifiers
- proof format changes
- verifier behavior changes

Rotation remains deferred according to:

`docs/design/ADR_SIGNING_KEY_GOVERNANCE.md`

---

## Result

Stage 14.1 validation confirms that signing key governance documentation was added without runtime, cryptographic contract, or verification behavior changes.
