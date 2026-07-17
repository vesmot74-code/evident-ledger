# Stage 4 §3 — failure_signal Policy Design

Date: 2026-07-17

Status: **design only** — no implementation in this document.

Scope reference: `docs/audits/API_V1_AUDIT.md` (post-implementation verification).

Explicitly **out of scope** for this design:

- retry policy
- timeout-based transitions
- background workers / job queue
- TSA queue / async stamping
- automatic repair

---

## B.1 Current State Machine

### Enum definition

File: `src/api/v1/proof_status.rs`

| Variant   | Wire value | Definition location |
| --------- | ---------- | ------------------- |
| `Pending` | `"pending"` | lines 12–16 |
| `Anchored`| `"anchored"` | lines 12–16 |
| `Failed`  | `"failed"` | lines 12–16 |

### ProofContext inputs (assembled upstream)

File: `src/api/v1/proof_status.rs` lines 29–36

| Field                 | Meaning |
| --------------------- | ------- |
| `merkle_root_present` | Prefix non-empty and recomputed root non-empty |
| `signature_present`   | Non-empty signature string |
| `signature_valid`     | `verify_root(...)` returned true |
| `failure_signal`      | Explicit failure flag (always `false` in production paths today) |

TSA is **not** part of `ProofContext` (by design — see `docs/API.md` §4).

### Pure derivation function

File: `src/api/v1/proof_status.rs` lines 64–74 — `derive_proof_status(ctx)`

```
if failure_signal        → Failed
else if merkle ∧ sig ∧ sig_valid → Anchored
else                     → Pending
```

Unit tests document intended transitions (`src/api/v1/proof_status.rs` lines 94–170):

| Input (merkle, sig_present, sig_valid, failure_signal) | Output |
| ------------------------------------------------------ | ------ |
| default / all false | Pending |
| merkle only | Pending |
| sig only (valid or invalid) | Pending |
| merkle + sig + invalid sig | **Pending** (not Failed) |
| merkle + sig + valid sig | Anchored |
| any + failure_signal=true | Failed |

### Where ProofStatus is computed or assigned

| Location | Operation | Typical outcome today |
| -------- | --------- | --------------------- |
| `src/api/v1/proof_material.rs:295` | `derive_proof_status(&snapshot.context)` on GET proof | Pending / Anchored / Failed (Failed unreachable: `failure_signal` hardcoded false) |
| `src/api/v1/proof_material.rs:96–106` | `build_proof_snapshot` (commit-time sign) | Always sets `failure_signal = false`; valid sig if prefix non-empty |
| `src/api/v1/proof_material.rs:139–155` | `build_proof_snapshot_read` (read path) | `failure_signal = false`; invalid persisted sig → `signature_valid=false` → **Pending** |
| `src/api/v1/submit_event.rs:93` | `derive_proof_status` after commit-time snapshot | **Anchored** for normal prefix; Pending if empty prefix |
| `src/api/v1/proof_material.rs:244,269,305` | Response JSON `"proof_status"` string literals | Pending / Anchored / Failed envelopes |
| `src/api/v1/submit_event.rs:73` | POST response `proof_status` field | From derived status at commit |

### Actual runtime transitions (implementation, not contract)

```
POST /v1/events (happy path):
  insert event → sign prefix → persist signature → derive → anchored (or pending if empty prefix)

GET /v1/proof/{event_id}:
  load prefix + persisted signature → recompute merkle → verify_root
    → signature empty        → pending
    → signature invalid      → pending   ← gap vs proposed FAILED policy
    → signature valid        → anchored
    → failure_signal=true    → failed envelope (handler exists; signal never set)

GET /v1/verify/{event_id}:
  stub only — no proof_status gating yet
```

Legacy `POST /events` writes `signature = ""` → v1 GET proof returns **pending** (not failed).

---

## B.2 Error Suppression Points

Command references (2026-07-17):

```text
grep -rn "verify_root" src/
  src/api/v1/proof_material.rs:29,100,140
  src/bin/verify.rs:10,114
  src/signing.rs:47

grep -rn "Result<.*ProofStatus" src/
  src/api/v1/submit_event.rs:85  (returns Result<(ProofStatus, String), ApiError>)
```

### 1. Invalid signature silently → Pending

| File | Lines | Condition | Current behavior |
| ---- | ----- | --------- | ---------------- |
| `src/api/v1/proof_material.rs` | 137–146 | `signature_present && !verify_root(...)` | `signature_valid = false`; `failure_signal = false` |
| `src/api/v1/proof_status.rs` | 64–73 | `merkle + sig_present + !sig_valid` | Returns **Pending** (test `merkle_with_invalid_signature_is_pending`, line 120) |

Confirmed: audit note matches code — invalid persisted signature does **not** produce `failed`.

### 2. Merkle recompute mismatch

There is **no persisted merkle_root column** on `events`. Merkle root is always recomputed from the prefix at read/commit time (`MerkleTree::recompute_root_from_events` in `proof_material.rs:92–93,133–134`).

Implication:

- A tampered chain (events altered after signing) changes recomputed merkle → `verify_root` fails → treated as invalid signature → **Pending** today.
- There is no separate code path comparing “stored root vs recomputed root”; mismatch manifests only through signature verification failure.

### 3. Malformed proof material

| Failure mode | Where handled | Effect on status |
| ------------ | ------------- | ---------------- |
| Malformed signature hex | `src/signing.rs:54–55` (`hex::decode` fails) → `verify_root` returns false | Pending (via invalid sig) |
| Malformed public key hex | `src/signing.rs:57–58,63–64,66–67` | Pending |
| Wrong signature length | `src/signing.rs:60–61` | Pending |
| Empty event prefix | `merkle_root_present = false` | Pending |
| DB / load errors | `build_proof_response` → `ApiError::Internal` (500) | HTTP error, not proof_status |

No branch sets `failure_signal` or `Failed` for malformed-but-present material.

### 4. TSA material present but invalid

| Location | Behavior |
| -------- | -------- |
| `src/api/v1/proof_material.rs:213–237` | Loads TSA row metadata only (`timestamp`, `serial`, `token_bytes` length); **no validation** |
| `src/api/v1/proof_material.rs:310–312` | TSA attached to anchored response after status already Anchored |
| `src/tsa/verify.rs:16–33` | Full TSA validation exists (`verify_tsa_attestation`) but **not called** from v1 proof/verify handlers |

TSA validation failure does not affect `proof_status` anywhere in v1 API today.

### 5. failure_signal hardcoded off

| File | Line | Comment |
| ---- | ---- | ------- |
| `src/api/v1/proof_material.rs` | 108 | `failure_signal = false; // Stage 3+: persisted failure sources` |
| `src/api/v1/proof_material.rs` | 148 | `failure_signal = false; // Stage 4 §3: failure_signal sources` |

The `Failed` response envelope in `build_proof_response` (lines 299–307) is implemented but unreachable in production.

---

## B.3 Proposal (design only)

### Design decision: derived failure_signal (no new DB column)

Stage 4 §3 should populate `ProofContext.failure_signal` **at assembly time** from deterministic verification rules, without a new migration. Rationale:

- `failure_signal` already exists on `ProofContext` with priority over Anchored.
- No `failure_reason` or `proof_failed_at` column exists today.
- Persisting failure state would require schema + backfill policy (deferred).

Optional future work (not §3): persist failure reason for audit — separate design.

### Failure conditions (proposed)

**FAILED** when any **runtime-checked** condition (1, 2, or 4 below) holds.

#### Runtime-checked failure conditions

1. **Signature present and verification fails** — `signature_present && !signature_valid`
   - Covers: wrong sig, tampered chain after sign, malformed hex, merkle/signature message mismatch.
2. **Merkle root cannot be computed from stored events but signature is non-empty**
   - e.g. empty prefix + non-empty signature (inconsistent persisted material).
4. **TSA row exists for recomputed merkle root but token validation fails**
   - Only when `tsa_tokens` row exists for `(chain_id, merkle_root)` AND `verify_tsa_attestation` returns `TsaStatus::Failed`.
   - `tsa == null` (no row) remains non-failure — unchanged.
   - **Note:** this introduces a **new call** to `verify_tsa_attestation` on the `GET /v1/proof` read path (not currently invoked there). Performance impact on hot-path reads is not evaluated in this design and should be a follow-up consideration before implementation.

#### Assumed invariants (not runtime-checked in §3 scope)

3. **Persisted signature is verified against the target event as `chain_head`**
   - Today `build_proof_snapshot_read` always passes `target_event_id` as `chain_head` (`proof_material.rs:136–137`). A mismatch would indicate a bug in assembly, not a client-visible proof-verification failure path.
   - **If violated, this indicates a bug elsewhere in the system, not an expected failure path. No test required for this condition in §3.**

Document as a code comment or `debug_assert!` at assembly if desired; do **not** implement as a runtime `failure_signal` source in §3.

**PENDING** remains for (unchanged from current intent):

- `signature` empty / not yet generated (legacy events, in-flight commit).
- Required material incomplete: merkle not present, signature not present.
- Async anchor step not completed — **not** modeled as pending-vs-failed distinction in §3 (no timeout logic).

**ANCHORED** when:

- `merkle_root_present && signature_present && signature_valid && !failure_signal`
- TSA absence does not block anchored (unchanged).

### Mapping change in `derive_proof_status` (decision)

**Option A (chosen):** Set `failure_signal = true` in `build_proof_snapshot_read` / assembly when runtime conditions 1, 2, or 4 hold; keep `derive_proof_status` pure.

Option B (rejected): extending `derive_proof_status` to return `Failed` for `signature_present && !signature_valid` directly — not used; would bypass the explicit `failure_signal` gate.

### Endpoint behavior (proposed)

| Endpoint | `proof_status` | HTTP | Body |
| -------- | -------------- | ---- | ---- |
| `GET /v1/proof` | pending | 200 | Minimal pending envelope (§6 — unchanged) |
| `GET /v1/proof` | anchored | 200 | Full proof artifact (unchanged) |
| `GET /v1/proof` | failed | 200 | Minimal failed envelope (`proof_status: "failed"`) — handler already exists at `proof_material.rs:299–307`; align with §6 doc gap in follow-up |
| `GET /v1/verify` | pending | 409 | `proof_not_ready` |
| `GET /v1/verify` | failed | 422 | `proof_generation_failed` |
| `POST /v1/events` | failed | — | **Should not occur** on synchronous commit path if signing is atomic; if signing fails → 500 `internal_error`, not `proof_status: failed` in 200 body |

### Code changes required

| File | Change |
| ---- | ------ |
| `src/api/v1/proof_material.rs` | In `build_proof_snapshot_read` (and optionally `build_proof_snapshot`): compute `failure_signal` from runtime conditions 1, 2, 4 instead of hardcoded `false`. Add helper e.g. `detect_failure_signal(ctx, tsa_validation)` to keep assembly readable. |
| `src/api/v1/proof_material.rs` | In `build_proof_response`: run TSA validation when row exists; pass result into failure detection before `derive_proof_status`. **New invocation** of `verify_tsa_attestation` on proof read path (see condition 4 note above). |
| `src/api/v1/proof_status.rs` | Update unit test `merkle_with_invalid_signature_is_pending` → rename to `merkle_with_invalid_signature_is_failed`, expect **Failed** (per Option A, see Mapping decision above). Add tests for runtime conditions 1, 2, 4. |
| `src/api/v1/proof.rs` | No route change; documents comment at line 23 may need update if failed ≠ pending semantically. |
| `src/api/v1/submit_event.rs` | No change expected — commit path always produces valid signature or errors with 500. |
| `src/api/v1/errors.rs` | Add `ApiError::ProofNotReady` / `ProofGenerationFailed` variants OR map to existing codes when verify is implemented (Stage 5). |
| `src/api/v1/verify.rs` | When verify is implemented: gate on derived status before verification body — pending → 409, failed → 422 (per `docs/API.md` §7). |
| `src/signing.rs` | No change — `verify_root` bool API sufficient. |
| `src/tsa/verify.rs` | Reuse `verify_tsa_attestation` from proof assembly; no change to TSA module itself. |

### API changes

| Document | Impact |
| -------- | ------ |
| `docs/API.md` §4 enum | Already defines `failed` — **no change required** for enum |
| `docs/API.md` §6 GET proof | **Gap:** documents pending + anchored only; does not show failed envelope. Follow-up doc sync should add failed response example (HTTP 200, minimal body) — **not in §3 implementation commit** per frozen-docs rule unless explicitly requested |
| `docs/API.md` §7 verify | Already defines `proof_not_ready` (409) and `proof_generation_failed` (422) — **aligns** with proposed verify gating |
| `docs/API.md` §2 error codes | Codes are lowercase snake_case (`proof_generation_failed`) — matches `ApiError` style; new variants needed at implementation |

Contract note: §7 failed behavior applies to **verify**; §6 failed body shape should mirror pending envelope with `"proof_status": "failed"` (already implemented in handler).

### Tests required (list only — do not implement here)

1. **GET proof — invalid persisted signature** → `proof_status: "failed"` (not pending).
2. **GET proof — legacy event (`signature=""`)** → still `pending`.
3. **GET proof — valid persisted signature** → `anchored` (regression).
4. **GET proof — tampered prefix events** (signature no longer verifies) → `failed`.
5. **GET proof — empty prefix, non-empty signature** → `failed` (malformed material).
6. **GET proof — TSA row present, invalid token** → `failed` (once TSA validation wired).
7. **GET proof — TSA row absent** → `anchored` if sig valid (regression).
8. **POST /v1/events** — still returns `anchored` on happy path (regression).
9. **derive_proof_status unit tests** — rename `merkle_with_invalid_signature_is_pending` → `merkle_with_invalid_signature_is_failed`, expect Failed; keep failure_signal priority tests.
10. **GET verify (Stage 5)** — failed status → 422 + `proof_generation_failed`; pending → 409 + `proof_not_ready`; ownership before status check (regression).

Integration tests likely extend `tests/v1_proof.rs`; verify tests in future `tests/v1_verify.rs`.

### Migration required: **no**

Failure detection is **derived at read/assembly time** from existing columns:

- `events.signature`
- `events` prefix rows (merkle recompute)
- `tsa_tokens` (optional validation)

No new table or column required for §3 as scoped.

If future work persists failure reason (`failure_signal` column or `proof_failure_code` on `events`), that would be a separate migration design — not part of this stage.

---

## Summary

| Item | Current | Proposed |
| ---- | ------- | -------- |
| Invalid sig on read | Pending | **Failed** |
| `failure_signal` in production | Always false | Set from verification rules (runtime conditions 1, 2, 4) |
| TSA invalid when row exists | Ignored for status | **Failed** (requires new `verify_tsa_attestation` call on GET proof read path; performance not evaluated here) |
| Merkle mismatch | Surfaces as invalid sig → Pending | **Failed** (via condition 1) |
| DB migration | — | **None** |
| Verify gating | Not implemented | 409 / 422 when verify built (Stage 5) |

Next implementation step after approval: code changes in `proof_material.rs` + test updates, then verify endpoint wiring in Stage 5.
