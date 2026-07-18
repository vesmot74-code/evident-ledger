# Evident Ledger — Verification Model

This document freezes the trust model for private verification (`/v1/verify`) and the public verification layer (`/public/verify`) as implemented through Stage 6.6 and documented in Stage 7.
It describes verification layers, disclosure rules, and architectural boundaries.
**It does not define runtime behavior by itself** — the implementation in `src/api/` is authoritative.

**Related documents:**

- [SECURITY.md](../SECURITY.md) — threat model, security assumptions, public API guarantees, security controls, and **security invariants** (architectural truths that survive implementation changes).
- [SYSTEM_CONTRACT.md](../SYSTEM_CONTRACT.md) — ownership model, public/private API boundary, materialization, and system-wide contracts.

---

## 1. Verification Pipeline

`GET /v1/verify/{event_id}` executes checks in this order:

```
Request
 |
 v
Authentication
 |
 v
Ownership Check
 |
 +---- missing/foreign → 404 NOT_FOUND
 |
 v
Query file_hash validation (if provided)
 |
 +---- invalid format → 400 INVALID_REQUEST
 |
 v
Proof Status Resolution
 |
 +---- Pending → 409 PROOF_NOT_READY
 |
 +---- Failed → 422 PROOF_GENERATION_FAILED
 |
 +---- Anchored
          |
          v
     Chain Verification
          |
          v
     File Hash Verification
          |
          v
       Response (200)
```

### Pipeline rules

- **`proof_status` is a gate.** `chain{}` and `file{}` run only after `ProofStatus::Anchored`.
- **Request errors are not masked by resource state.** `400` (invalid `file_hash` format) and `404` (ownership) are returned regardless of whether proof is Pending or Failed.
- **Ownership runs before query validation.** A foreign or missing `event_id` returns `404` before invalid `file_hash` format is evaluated — preventing existence side-channels through error type.
- **Query validation runs before proof gating.** An invalid `file_hash` format returns `400` even when proof is Pending or Failed.

Implementation: `src/api/v1/verify.rs` (`V1Auth` → `verify_event_access` → `normalize_query_file_hash` → `resolve_proof_state` → `verify_chain_prefix` → `verify_file_hash`).

### HTTP status codes — `/v1/verify`

| Scenario | HTTP | `error.code` |
|----------|------|--------------|
| Missing or invalid `X-API-KEY` | 401 | `unauthorized` |
| Missing or foreign `event_id` | 404 | `not_found` |
| Invalid `file_hash` format | 400 | `invalid_request` |
| Proof Pending | 409 | `proof_not_ready` |
| Proof Failed | 422 | `proof_generation_failed` |
| Proof Anchored (chain/file checks complete) | 200 | — |

**401 note:** `V1Auth` maps any authentication failure to `ApiError::Unauthorized` — HTTP 401, body `{ "error": { "code": "unauthorized", "message": "Missing or invalid API key", "request_id": "..." } }`. This applies to all v1 endpoints using `V1Auth`, not only verify.

A mismatching but **well-formed** `file_hash` is not an error — it returns HTTP 200 with `file.is_valid_file_hash = false`.

---

## 2. Three-layer Trust Model

Verification is layered. Each layer answers a distinct question.

### Layer 1 — Proof State

**Purpose:** Determine whether proof material is ready for the event.

**Source:** `resolve_proof_state()` (`src/api/v1/proof_state.rs`)

**Statuses:** `Pending`, `Failed`, `Anchored`

| Status | Meaning | HTTP (verify) |
|--------|---------|---------------|
| `Pending` | Proof not yet available | 409 `proof_not_ready` |
| `Failed` | Proof generation or integrity signal failed | 422 `proof_generation_failed` |
| `Anchored` | Proof ready — continue to chain and file layers | 200 (if later checks pass) |

Proof state is resolved from persisted signature, TSA material, and failure signals. It gates access to structural verification — handlers do not reach `chain{}` or `file{}` until Anchored.

---

### Layer 2 — Chain Integrity

**Purpose:** Verify that the event's chain prefix has not been tampered with.

**Source:** `verify_chain_prefix()` (`src/api/v1/chain_verification.rs`)

**Response shape (`chain{}`):**

```json
{
  "valid": true,
  "merkle_valid": true,
  "signature_valid": true,
  "errors": []
}
```

**Semantics:**

- Verification covers the **prefix** ending at the target event — not the full chain and not future events.
- The **expected root** comes from proof state resolution (`resolved_root` / snapshot merkle root at resolve time).
- The persisted **signature** is verified against that expected root.
- The **recomputed merkle root** from prefix events is compared to the expected root.
- `chain.valid` is the aggregate; `merkle_valid` and `signature_valid` are independent structural checks.

**Architectural constraint (private API — Stage 5.4):**

In the current `/v1/verify` pipeline, `chain.merkle_valid = false` and `chain.signature_valid = false` are **practically unreachable via HTTP**. Most data-corruption scenarios that would produce false integrity flags are intercepted earlier by `detect_failure_signal` → `ProofStatus::Failed` → HTTP 422, before the handler reaches `verify_chain_prefix()`.

`verify_chain_prefix()` is covered by unit and direct integration tests. The false branches exist in the algorithm for defense-in-depth and future divergence, but they are not HTTP-reachable under the current private pipeline.

**Future public API:**

For `/public/verify` (without proof-status gating), reusing `verify_chain_prefix()` will make these false branches **HTTP-reachable for the first time**. This is intentional: **the public layer has a wider spectrum of reachable verification states than the private layer.**

---

### Layer 3 — File Verification

**Purpose:** Compare a caller-provided file hash claim against the stored registration hash.

**Source:** `verify_file_hash()` (`src/api/v1/file_verification.rs`)

This is **not** a check of a physical file. The system has no access to the original file. A positive result means the provided hash matches what was registered — not that the file exists or is intact on disk.

**Response shape (`file{}`):**

```json
{
  "provided": true,
  "provided_hash": "sha256...",
  "is_valid_file_hash": true
}
```

| Field | When absent query | When provided, matches | When provided, mismatch |
|-------|-------------------|------------------------|-------------------------|
| `provided` | `false` | `true` | `true` |
| `provided_hash` | `null` | echo of query param | echo of query param |
| `is_valid_file_hash` | `null` | `true` | `false` |

**Rules:**

- The stored `event.file_hash` is used **only inside** `verify_file_hash()` for comparison — it is never serialized, logged in responses, or returned to the caller.
- There is **no** `file.status` enum — the contract uses `provided` + `is_valid_file_hash` only, avoiding a second source of truth.
- Query input is normalized (`trim()` + lowercase) and validated (64 hex chars, `[0-9a-f]`) before comparison.

---

## 3. Zero Disclosure Principle

> Verification API does not reveal internal stored evidence identifiers or stored file hashes unless explicitly included in a future public disclosure contract.

**Forbidden response fields:**

```
expected_hash
stored_hash
stored_file_hash
```

The caller receives only:

- Their own submitted hash (`provided_hash` — echo of the query parameter)
- The comparison result (`provided`, `is_valid_file_hash`)

Nothing the caller did not provide is exposed by default.

---

## 4. Private vs Public Verification Boundary

### Private API — `/v1/verify`

**Purpose:** Owner verifies their own registered event.

**Requires:** `X-API-KEY`, `event_id`

**Checks:** authentication → ownership → optional file hash claim → proof state → chain prefix → file hash claim

**Scope ends here for Stage 5.x.** This document defines the private verification contract through Stage 5.4.

---

### Future Public API — `/public/verify` *(superseded — see Public Verification Model below)*

**Purpose:** Verify existence of a proof by file hash (no ownership).

**Input (planned):** `file_hash`

**Not in scope for Stage 5.5.** Design and implementation are Stage 6.

---

### Public Verification Model

#### Canonical Proof Selection

For a given file hash, the public verification layer exposes only one canonical proof.

Canonical selection rule:

```
canonical proof =
    proof
    WHERE:
        status = Anchored
        AND enabled = true
    ORDER BY:
        created_at ASC
    LIMIT 1
```

Properties:

- Multiple internal proofs MAY exist for the same file hash.
- Internal proof history remains unchanged.
- Public verification MUST NOT expose proof count or internal proof relationships.
- The same file hash MUST resolve to the same canonical public proof while the selected proof remains enabled.

#### Canonical Proof Immutability (first-materialized-wins)

Once a canonical public proof is materialized for a `file_hash` (first successful row in `public_proofs` with `enabled = true`), that binding is **fixed** while `enabled = true`.

- The `ORDER BY created_at ASC` rule applies **only** when selecting the first candidate at initial materialization — not on every request, and not after an active canonical row already exists for that hash.
- A later internal proof for the same `file_hash` becoming `Anchored` does **not** replace the canonical public proof (first-materialized-wins).
- `public_id` is assigned **once** at materialization — not recomputed per request. This preserves "same hash → same public proof while enabled" without re-evaluating canonical selection on each lookup.

Re-materialization runs only when no active canonical row exists (never materialized, or previous canonical was disabled with `enabled = false`). Proofs explicitly disabled (`enabled = false` row for a given `proof_id`) are not auto-reactivated.

The canonical proof is deterministic given materialization history; it is not re-derived from live database ordering on each call.

#### Status Visibility Rule

Public verification exposes only externally verifiable existence.

Visibility rules:

```
Anchored  → verified = true
Pending   → verified = false (no disclosure)
Failed    → verified = false (no disclosure)
Missing   → verified = false
```

The public API MUST NOT distinguish between:

- proof does not exist;
- proof exists but is pending;
- proof exists but failed;
- proof existed but is no longer publicly enabled.

Reason: internal lifecycle state is private metadata.

Public verification answers only: "Is there a currently valid public proof for this hash?"

It does not answer: "Was this hash previously submitted?"

#### Public Proof Enablement

`enabled` controls public visibility of a proof.

```
enabled = true  → eligible for canonical selection
enabled = false → excluded from public verification
```

Disabling a public proof MUST NOT reveal historical existence.

Public response after disabling:

```json
{
  "verified": false
}
```

No additional status MUST be returned.

The `enabled` flag is an internal visibility control and is not a public revocation signal.

---

Public verification is an existence proof interface, not an audit history interface.

---

### Public Verification Security Boundary

#### Public Lookup Source

Public verification MUST query only the public proof registry.

The public verification layer MUST NOT access:

- events;
- chains;
- internal proof state;
- account data;
- private evidence metadata.

The public proof registry is a materialized public projection.
It is not the source of truth for evidence integrity.

#### Existence Disclosure Rule

Public verification exposes only whether a currently enabled public proof exists.

The public API MUST NOT reveal:

- submission history;
- pending state;
- failed state;
- disabled public proofs;
- internal lifecycle transitions.

Public verification answers only:
"Is there a currently valid public proof?"

It does not answer:
"Was this hash ever submitted?"

#### Response Contract

Found and not found responses MUST:

- use the same HTTP status code;
- use the same JSON schema;
- differ only by public verification values.

The public verification layer MUST NOT use HTTP status differences
to indicate proof existence.

Expected behavior:

- `200 + verified=true`
- `200 + verified=false`

#### Lookup Timing Rule

Public verification MUST minimize observable timing differences
between verified and not verified responses.

Found and not found paths MUST use the same lookup flow.

Implementation requirement — a single public registry lookup:

```sql
SELECT public_id
FROM public_proofs
WHERE file_hash = $1
  AND enabled = true
```

MUST NOT:

- perform existence lookup first and metadata lookup second;
- return early on not-found before completing the public lookup flow;
- access internal evidence tables for found cases only.

The public lookup path must remain derived only from the public registry.

#### Hash Validation

Before public registry lookup:

1. Validate SHA256 format.
2. Validate expected length (64 characters).
3. Validate hexadecimal encoding.
4. Normalize hexadecimal characters to lowercase.
5. Execute public registry lookup using normalized hash.

Invalid hash → `400 Bad Request`, `{"error": "invalid_hash"}`.

Invalid hashes MUST NOT execute database lookup.

Write-side precondition: `file_hash` values written via `on_proof_anchored()`
MUST be normalized to lowercase hex at write time so read-side normalization
matches stored registry keys.

#### Forbidden Disclosure Fields

Public verification responses MUST NOT contain:

- event_id;
- chain_id;
- account_id;
- internal proof_id;
- sequence numbers;
- merkle roots;
- signatures;
- internal timestamps;
- storage paths;
- database identifiers.

Internal identifiers MUST NOT be exposed through:

- JSON response body;
- HTTP headers;
- generated public URLs.

#### Timing Disclosure

Found and not found verification paths MUST minimize observable
timing differences.

The implementation MUST avoid unnecessary branching that exposes
internal lookup state.

See **Lookup Timing Rule** under Response Contract above.

If database-level constant-time lookup is not available,
the residual timing risk MUST be documented.

#### Rate Limit Policy

Public verification is anonymous.
No API key is required.

Rate limiting is enforced using network-level controls.

Minimum protection:

- per-IP request limiting;
- burst protection;
- abuse monitoring.

IP-based limiting alone is not considered a complete
anti-enumeration solution.

#### Hash Enumeration Threat

Public verification MUST assume that attackers may attempt
large-scale hash existence queries.

The system MUST NOT expose additional metadata that increases
the value of enumeration attempts.

---

## 5. Reuse Rule for Future Public API

> The future public layer **must not implement its own verification logic**.

**Required reuse:**

```
resolve_proof_state()
verify_chain_prefix()
verify_file_hash()
```

**Forbidden:**

- A new proof resolver
- A separate merkle verification path
- A separate hash validation path

Public and private layers may differ in **access control** (no `X-API-KEY`, lookup by hash only) but **must not diverge in verification semantics**.

---

## 6. Prefix Semantics

Evident Ledger verifies the chain prefix up to and including the target event:

```
Genesis
  |
Event 1
  |
Event 2
  |
Target Event  ← verification point
```

**Not verified:**

```
Target Event
  |
Future events
```

**Reason:** Events appended after the target cannot retroactively invalidate proof material already committed for that event. Prefix verification captures the integrity state at the registration point.

---

## 7. Module Map

| Concern | Module | Pure function |
|---------|--------|---------------|
| Authentication | `src/api/v1/auth.rs` | — (extractor) |
| Ownership | `src/api/v1/event_access.rs` | `verify_event_access()` |
| Query validation | `src/api/v1/file_verification.rs` | `normalize_query_file_hash()` |
| Proof gating | `src/api/v1/proof_state.rs` | `resolve_proof_state()` |
| Chain integrity | `src/api/v1/chain_verification.rs` | `verify_chain_prefix()` |
| File claim | `src/api/v1/file_verification.rs` | `verify_file_hash()` |
| Handler orchestration | `src/api/v1/verify.rs` | — |

---

## Document status

**Frozen at Stage 7 (Architecture Freeze).**

Changes to verification semantics, public disclosure rules, or trust boundaries require an explicit stage and updates to this document, [SECURITY.md](../SECURITY.md) (§2.5 invariants), and [SYSTEM_CONTRACT.md](../SYSTEM_CONTRACT.md) where ownership or API boundaries are affected — per Security Invariant 11.

For implementation details of the public perimeter (Stages 6.3–6.6): existence-only responses, rate limiting, format validation, unified query path, and public verification telemetry — see [SECURITY.md](../SECURITY.md) §2.3–2.4.
