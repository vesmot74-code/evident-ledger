# Stage 2.4 Audit — Lifecycle Persistence Hardening

**Date:** 2026-08-10  
**Base:** Stage 2.3 (`v1.3.0-stage2.3`) — `evident verify --event … --chain …`  
**Scope:** Monotonicity + idempotent persistence of `lifecycle_status`. No schema / CLI / PDF / QR / GUI changes.

---

## Lifecycle Monotonicity

### Finding

`refresh_lifecycle` previously assigned:

```text
record.lifecycle_status = derive_lifecycle(integrity, tsa_status)
```

`derive_lifecycle` **ignores** the prior status. On a later integrity/TSA regression it could return `REGISTERED` even when the record was already `CERTIFIED` — a real downgrade path if `refresh_lifecycle` is called with a tampered or incomplete proof.

CLI Stage 2.3 already avoided writing on failed integrity (`IntegrityFailed` without `refresh_lifecycle`), but any direct `refresh_lifecycle` call remained unsafe.

### Change

Minimal fix in `src/evidence_record.rs`:

- Added `advance_lifecycle(current, derived)` — keep the further-along status on the ladder `CREATED → REGISTERED → TSA_CONFIRMED → CERTIFIED`.
- `REVOKED` is sticky (reserved; never produced by `derive_lifecycle`).
- `refresh_lifecycle` now: derive → `advance_lifecycle(current, derived)`.

`derive_lifecycle` / `verify_evidence_integrity` / crypto unchanged.

---

## Persistence Behavior

| Step | Behavior |
| --- | --- |
| When `write_evidence_record` runs | Only on **successful** integrity in `verify_event_evidence` (after `refresh_lifecycle`). |
| Failed integrity | No lifecycle write; report returned as `IntegrityFailed`. |
| Repeated successful verify | `refresh_lifecycle` keeps `CERTIFIED`; `write_evidence_record` runs again with the **same** JSON fields. |
| File mtime | May change on rewrite; content is idempotent (no field drift). |
| `last_verified_at` / counters | **Not** present; **not** added in Stage 2.4. |

---

## Test Coverage

| Test | Location | Isolation |
| --- | --- | --- |
| `test_verify_certified_is_idempotent` | `src/event_evidence_verify.rs` | `tempdir()` + copy of `tests/fixtures/stage2_3/{evidence,proof}.json` (event `22d29a6a-…`, chain `c0bafd33-…`) |
| `test_certified_lifecycle_never_downgrades` | `src/evidence_record.rs` | Synthetic signed proof in `tempdir()`; also asserts no downgrade after signature tamper |

Does **not** mutate `~/.evident/`.

Scenarios covered:

1. First verify → `CERTIFIED`, second verify → still `CERTIFIED`, JSON equal.
2. `CERTIFIED` + valid refresh → stays `CERTIFIED`.
3. `CERTIFIED` + failing integrity on refresh → stays `CERTIFIED` (no `TSA_CONFIRMED` / `REGISTERED` rollback).

---

## Deferred Items (Stage 3+)

| Item | Why deferred |
| --- | --- |
| `last_verified_at` / `verified_count` | Would change EvidenceRecord schema; out of Stage 2.4 scope. |
| Audit history / DB lifecycle journal | Server/DB concern; not local projection hardening. |
| Skip disk write when bytes unchanged | Optimization only; current rewrite is content-safe. |
| CLI interface changes | Stage 2.3 frozen. |
