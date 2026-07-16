# Public API v1 — Implementation Roadmap

Status: Draft  
Scope: Backend HTTP API only (documentation; no code changes in this document)

## Source of truth

This plan is subordinate to:

- [SYSTEM_CONTRACT.md](../SYSTEM_CONTRACT.md) — verification scope, trust model, `trust_level` enum, ownership, capabilities
- [docs/API.md](./API.md) — Public API Contract v0.1-draft Revision 2

Where this plan and the contracts disagree, the contracts win. This document must not introduce new endpoints, business rules, or architectural changes beyond what the contracts define.

## Current state (baseline)

The running server exposes legacy paths without the `/v1` prefix (`POST /events`, `GET /verify/{chain_id}`, `GET /account/capabilities`, etc.). Error responses are not yet unified under the `{ error: { code, message, request_id } }` envelope. Idempotency today uses a body field (`idempotency_key`) scoped to `chain_id`, not the HTTP `Idempotency-Key` header scoped per `account_id` as defined in API.md.

Implementation work below brings the backend into alignment with the v1 contract.

---

## API versioning

All Public API endpoints use the `/v1` prefix:

```http
POST /v1/events
GET  /v1/proof/{event_id}
GET  /v1/verify/{event_id}
GET  /v1/account/capabilities
```

**Rule:** All breaking API changes require a new version prefix.

```text
v1 → v2
```

The v1 contract is frozen after implementation and must remain backward compatible.

Legacy unprefixed routes may remain temporarily for internal or migration use, but they are **not** part of the Public API v1 contract and must not be documented as v1 endpoints.

---

## Implementation stages

### Stage 1 — Authentication and error contract

**Goal:** Every Public API v1 endpoint authenticates via `X-API-KEY`, resolves `account_id`, and returns errors in the unified envelope.

#### Authentication

All v1 endpoints require:

```http
X-API-KEY: <key>
```

Resolution: `AuthedAccount → account_id` (see SYSTEM_CONTRACT §13 and §16).

#### Error envelope

All API endpoints use a single format:

```json
{
  "error": {
    "code": "STRING",
    "message": "STRING",
    "request_id": "STRING"
  }
}
```

| HTTP | Code            |
| ---- | --------------- |
| 400  | INVALID_REQUEST |
| 401  | UNAUTHORIZED    |
| 403  | FORBIDDEN       |
| 404  | NOT_FOUND       |
| 409  | CONFLICT        |
| 422  | UNPROCESSABLE   |
| 429  | RATE_LIMITED    |
| 500  | INTERNAL_ERROR  |

Domain-specific codes (e.g. `PROOF_NOT_READY`, `PROOF_GENERATION_FAILED`) use the HTTP status defined in the relevant endpoint section of [docs/API.md](./API.md).

#### Deliverables

- [ ] Mount v1 router under `/v1`
- [ ] Middleware or handler wrapper that injects `request_id` and maps internal failures to the envelope
- [ ] Replace ad-hoc `{ "error": "string" }` responses on v1 routes
- [ ] Integration tests for 401 (missing/invalid key) on each v1 endpoint

---

### Stage 2 — Idempotency layer for `POST /v1/events`

**Goal:** Optional HTTP header deduplication per account, as specified in [docs/API.md §4](./API.md).

**Placement:** After Stage 1 (authentication and error contract), before Stage 3 (proof and verification endpoints).

#### Header

```http
POST /v1/events
Idempotency-Key: <uuid>
```

The header is **optional**. If omitted, no deduplication is performed; each request creates a new event.

#### Processing rules

**Replay (same request):**

When all of the following match:

- `account_id`
- `Idempotency-Key`
- request body (via `request_hash`; see below)

the server:

- does **not** create a new event;
- returns the stored response body.

**Conflict (different body, same key):**

When `account_id` and `Idempotency-Key` match but the request body differs:

```http
HTTP 409 CONFLICT
```

```json
{
  "error": {
    "code": "CONFLICT",
    "message": "",
    "request_id": ""
  }
}
```

**Scoping:** `Idempotency-Key` is scoped per `account_id`. The same key used by different accounts does not collide.

#### Storage requirement

Persist idempotency state in a dedicated store. Minimum model:

**Table:** `idempotency_records`

| Field            | Description                          |
| ---------------- | ------------------------------------ |
| `account_id`     | Owner account                        |
| `idempotency_key`| Value from `Idempotency-Key` header  |
| `request_hash`   | SHA-256 hex of canonical request body|
| `response_body`  | Serialized successful response       |
| `created_at`     | Record creation time                 |
| `expires_at`     | Expiration time                      |

**TTL:** 24 hours.

Unique constraint: `(account_id, idempotency_key)`.

#### `request_hash` definition

```text
request_hash = SHA-256 hex digest of the canonical JSON representation
of the request body (object keys sorted lexicographically,
no spaces or newlines).
```

**Note on canonicalization:** The codebase contains `freeze::canonical_json` (serde struct serialization for proof events). That function does **not** implement lexicographic key sorting for arbitrary JSON objects. Before implementation, choose and document the canonicalization approach (see [Open questions](#open-questions)).

#### Deliverables

- [ ] Extract `Idempotency-Key` from request headers (not request body)
- [ ] Compute `request_hash` per agreed canonicalization
- [ ] Upsert/lookup in `idempotency_records` before event creation
- [ ] Return stored response on replay; 409 on conflict
- [ ] Expire records after 24h (background job or lazy delete)

---

### Stage 3 — `POST /v1/events`

**Goal:** Event submission aligned with [docs/API.md §4](./API.md).

#### Request

```json
{
  "chain_id": "...",
  "file_hash": "...",
  "event_type": "commit"
}
```

#### Response

```json
{
  "event_id": "...",
  "chain_id": "...",
  "sequence": 5,
  "proof_status": "anchored",
  "trust_level": "BASIC"
}
```

#### Enums (do not redefine)

- `proof_status`: `pending` | `anchored` | `failed` — [docs/API.md §4](./API.md)
- `trust_level`: `BASIC` | `ENHANCED` | `VAULT` | `IDENTITY` — SYSTEM_CONTRACT §14, §15

#### Deliverables

- [ ] v1 request/response schema separate from legacy `/events`
- [ ] Wire Stage 2 idempotency layer
- [ ] Map ledger errors to unified error envelope

---

### Stage 4 — Proof and verification endpoints

**Goal:** `GET /v1/proof/{event_id}` and `GET /v1/verify/{event_id}` with ownership enforcement and contract-accurate responses.

#### Ownership check (both endpoints)

Both endpoints **must** verify resource ownership:

```http
GET /v1/proof/{event_id}
GET /v1/verify/{event_id}
```

**Required check order:**

```text
1. Validate X-API-KEY
2. Resolve account_id
3. Check event ownership
4. Load proof
5. Execute operation
```

If `event_id` belongs to another account:

```http
HTTP 403 FORBIDDEN
```

```json
{
  "error": {
    "code": "FORBIDDEN",
    "message": "",
    "request_id": ""
  }
}
```

**Forbidden:**

- Checking another account's proof
- Returning another account's data
- Bypassing ownership via the verify endpoint

#### Ownership before `proof_status` on verify (§4.1)

For `GET /v1/verify/{event_id}`, ownership check **always** runs **before** `proof_status` check:

```text
1. Validate X-API-KEY
2. Resolve account_id
3. Check event ownership          → 403 if foreign event,
                                     regardless of proof_status
4. Load proof
5. Check proof_status             → 409/422 if pending/failed (see below)
6. Execute verification           (merkle / signature / chain)
```

**Rule:**

```text
Response code for a foreign event_id MUST always be 403,
regardless of that event's proof_status (anchored/pending/failed).
Checking proof_status before ownership is forbidden — it would let
an attacker distinguish proof states of events they don't own via
differing error codes (403 vs 409 vs 422), leaking information
about accounts they have no access to.
```

---

#### `GET /v1/proof/{event_id}`

Full response schema is defined in [docs/API.md §6](./API.md).

**Do not implement a shortened proof object.** Required fields:

```text
proof_version
proof_type
leaf_version
event_id
chain_id
sequence
parent_event_id
file_hash
merkle_root
signature
public_key
tsa
created_at
```

`tsa` is `null` when TSA state does not exist; the API must not fabricate TSA data.

---

#### `GET /v1/verify/{event_id}`

**Query parameter (only valid form):**

```http
GET /v1/verify/{event_id}?file_hash=<hex>
```

**Remove / do not implement:**

- `?v=file_hash`
- `?hash=`
- Any other alternate query parameter names

**Without `file_hash`:** Normal response; file verification not performed.

```json
{
  "chain": {
    "valid": true,
    "merkle_valid": true,
    "signature_valid": true,
    "errors": []
  },
  "file": {
    "status": "NOT_PERFORMED"
  }
}
```

`file.status = NOT_PERFORMED` is a normal state, not an error.

**With `file_hash=<hex>`:** Server compares submitted hash to proof hash:

| Result        | `file.status` |
| ------------- | ------------- |
| Hash matches  | `VALID`       |
| Hash differs  | `TAMPERED`    |

Chain verification and file verification are independent. The API must **not** expose a combined validity flag. See SYSTEM_CONTRACT §4.1 and §4.2.

**`file.status` enum:** `NOT_PERFORMED` | `VALID` | `TAMPERED` — do not extend.

#### `proof_status` behavior on verify

Applied **only after** successful ownership check (§4.1):

| `proof_status` | Behavior                                              |
| -------------- | ----------------------------------------------------- |
| `anchored`     | Normal verification (Merkle, signature, chain)          |
| `pending`      | HTTP 409, code `PROOF_NOT_READY`                      |
| `failed`       | HTTP 422, code `PROOF_GENERATION_FAILED`              |

**Forbidden:** Returning `chain.valid=true` when `proof_status` is `pending` or `failed`.

#### Deliverables

- [ ] `GET /v1/proof/{event_id}` with full schema from API.md §6
- [ ] `GET /v1/verify/{event_id}` with `?file_hash=<hex>` only
- [ ] Ownership gate on both endpoints (403 for cross-account)
- [ ] Verify handler order: ownership → proof_status → verification
- [ ] Chain/file response split per API.md §7

---

### Stage 5 — `GET /v1/account/capabilities`

**Goal:** Expose existing capabilities logic under the v1 path with contract-compliant errors.

Business logic unchanged. Capability fields and entitlement rules: SYSTEM_CONTRACT §13 (not duplicated here).

#### Requirements

- Mount at `GET /v1/account/capabilities`
- Errors use the unified error envelope (Stage 1)
- Verify HTTP status mapping for:
  - `401 UNAUTHORIZED` — missing or invalid API key
  - `403 FORBIDDEN` — valid key but no access (if applicable)
  - `500 INTERNAL_ERROR` — server failure

#### Deliverables

- [ ] v1 route wired to existing `get_account_capabilities`
- [ ] Error responses match API contract envelope
- [ ] Tests for 401 and 500 error shape

---

### Stage 6 — Backup API placeholder

**Do not implement** `GET /v1/backup/*` as a ready API.

```text
Backup API placeholder.
```

Concrete backup endpoints are **out of scope** for API v0.1-draft. They will be specified in a future revision after storage/export requirements are finalized.

Existing internal `/backup` routes (if any) remain outside the Public API v1 contract until a future API revision defines them.

---

## Data formats

All v1 responses and requests must use these formats (see also [docs/API.md §5](./API.md)):

### Cryptographic fields

```text
file_hash
merkle_root
signature
public_key
```

- Lowercase hexadecimal string
- No `0x` prefix

### Timestamps

```text
created_at
```

- ISO 8601 UTC
- Example: `2026-07-16T10:00:00Z`

### Sequence

```text
sequence
```

- Unsigned integer
- Monotonically increasing per `chain_id`

---

## Test plan

### Authentication and errors

- [ ] Each v1 endpoint returns 401 with unified envelope when `X-API-KEY` is missing or invalid
- [ ] Error responses include `code`, `message`, and `request_id`

### Idempotency

**Test: idempotent replay**

1. First request:

   ```http
   POST /v1/events
   Idempotency-Key: test-key-1
   ```

   Creates `event_id=A`.

2. Second request (same key, same body):

   ```http
   POST /v1/events
   Idempotency-Key: test-key-1
   ```

   **Expect:** `event_id=A`; no new event created.

**Test: idempotency conflict**

1. First request: `Idempotency-Key=test-key-1`, `file_hash=X`
2. Second request: `Idempotency-Key=test-key-1`, `file_hash=Y`

   **Expect:** HTTP 409, code `CONFLICT`.

**Test: idempotency scoped per account**

- Same `Idempotency-Key` on two different accounts with different bodies must **not** conflict.

### Ownership and verify

**Test: ownership vs proof_status precedence**

1. Account A creates an event with `proof_status=pending`.
2. Account B requests:

   ```http
   GET /v1/verify/{event_id belonging to A}
   ```

   **Expect:** HTTP 403 `FORBIDDEN` (not 409 `PROOF_NOT_READY`).

   Confirms ownership is checked before `proof_status`.

**Test: cross-account proof**

- Account B requests `GET /v1/proof/{event_id belonging to A}` → 403.

### Verify behavior

- [ ] No `file_hash` → `file.status = NOT_PERFORMED`, chain verification runs (when anchored)
- [ ] Matching `file_hash` → `VALID`
- [ ] Mismatching `file_hash` → `TAMPERED`
- [ ] `proof_status=pending` (own event) → 409 `PROOF_NOT_READY`
- [ ] `proof_status=failed` (own event) → 422 `PROOF_GENERATION_FAILED`
- [ ] No `chain.valid=true` for pending/failed

### Proof schema

- [ ] `GET /v1/proof/{event_id}` returns all required fields from API.md §6

### Capabilities

- [ ] `GET /v1/account/capabilities` returns 401/500 with unified error envelope

---

## Pre-implementation checklist

Before starting backend implementation, confirm:

- [x] All endpoints use `/v1` prefix
- [x] Error format defined
- [x] Ownership check defined
- [x] Ownership check strictly precedes proof_status check (§4.1)
- [x] Proof schema matches API.md
- [x] Verify uses only `?file_hash=<hex>`
- [x] Verify separates chain and file status
- [x] pending/failed proof behavior defined
- [x] Idempotency-Key defined (optional, scoped per account_id)
- [ ] request_hash canonicalization defined or logged as open question
- [x] Idempotency tests planned
- [x] Ownership-vs-proof_status precedence test planned
- [x] account/capabilities follows error contract
- [x] Backup wildcard marked out of scope
- [x] No undocumented endpoints added

---

## Canonical request hashing for Idempotency-Key

Status: Resolved

The Idempotency layer requires a deterministic request_hash calculation.

The purpose of request_hash is to identify whether repeated
POST /v1/events requests contain the same logical request body.

The hash calculation MUST satisfy:

- Same logical JSON request MUST produce the same request_hash.
- JSON key ordering MUST NOT affect the hash result.
- The algorithm MUST be deterministic across environments.
- The implementation MUST NOT depend on undefined JSON serialization order.

Decision:

The Idempotency layer MUST use a dedicated canonical JSON serializer,
designed specifically for request hashing, combined with SHA-256 to
produce request_hash as a lowercase hex digest
(consistent with the data format rules defined elsewhere in this document).

The serializer MUST:

- sort object keys deterministically (lexicographic order);
- produce stable output across runs and environments;
- preserve array element ordering as-is (no reordering of array items);
- generate identical output for identical logical requests regardless
  of input field order.

```text
request_hash = lowercase_hex(SHA-256(canonical_json(request_body)))
```

The existing `freeze::canonical_json` helper MUST NOT be reused for
Idempotency hashing. It was designed for a different subsystem, its
determinism guarantees are not part of the Idempotency contract, and
it is not confirmed to sort object keys lexicographically.

Reason:

A dedicated serializer removes ambiguity and prevents incorrect
duplicate-event creation caused by unstable or inconsistent request
hashes. Reusing a helper built for another subsystem's needs would tie
the Idempotency contract to guarantees that were never made for it.

---

## Open questions

### Verification rate limiting

See [docs/API.md §10](./API.md) — whether `/verify/{event_id}` is rate-limited per API key or per IP independently of commit quota.

---

## Out of scope (explicit)

This plan and v1 implementation must **not**:

- Add new Public API endpoints beyond API.md
- Change API contracts (`SYSTEM_CONTRACT.md`, `docs/API.md`)
- Replace `GET /v1/verify/{event_id}` with POST
- Modify enums: `proof_status`, `file.status`, `trust_level`
- Change storage model without separate agreement
- Implement concrete backup endpoints under v0.1-draft
