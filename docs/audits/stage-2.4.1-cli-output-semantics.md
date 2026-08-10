# Stage 2.4.1 Audit — CLI Output Semantics Fix

**Date:** 2026-08-10  
**Base:** Stage 2.4 (`v1.3.0-stage2.4`)  
**Scope:** CLI report formatting only (`src/event_evidence_verify.rs`). No lifecycle / crypto / schema changes.

---

## Problem

Stage 2.3/2.4 printed:

```text
Overall:              CERTIFIED
```

using the same value as `lifecycle_status` after (or before) refresh.

That conflated two different concepts:

| Concept | Meaning |
| --- | --- |
| Historical lifecycle | Highest state the certificate has reached (`CREATED`…`CERTIFIED`), monotonic |
| Current verification run | Whether **this** `verify_evidence_integrity` call passed |

A historically `CERTIFIED` record with a later tampered signature could show:

```text
Signature Valid:      FAIL
Overall:              CERTIFIED
```

Technically consistent with Stage 2.4 monotonicity, but easy to misread as “this run passed.”

---

## Decision

| Field | Source | Meaning |
| --- | --- | --- |
| `Verification Result` | `EvidenceIntegrityResult` only (`event_found` ∧ parent ∧ merkle ∧ signature ∧ `errors.is_empty()`) → `PASS` / `FAILED` | Outcome of **this** verify |
| `Lifecycle` | Persisted / reported `lifecycle_status` | Historical certificate state |

Removed ambiguous `Overall:` entirely.

`advance_lifecycle` / `refresh_lifecycle` / exit codes unchanged.

---

## Verification

- `Verification Result` never reads `lifecycle_status`.
- Failed integrity still leaves `Lifecycle: CERTIFIED` when the record was already certified (CLI does not write on integrity failure; refresh is monotonic if called).
- Exit codes: missing/corrupt → 1; integrity fail → 2; success → 0 (unchanged).

Tests: successful fixture PASS + CERTIFIED; post-certify signature tamper → FAIL + FAILED + CERTIFIED.
