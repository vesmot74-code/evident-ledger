# API.md — Evident Ledger Public API

Version: 0.1-draft  
Status: Draft — not yet implemented

Companion document: [SYSTEM_CONTRACT.md](../SYSTEM_CONTRACT.md)

SYSTEM_CONTRACT.md is the source of truth for verification scope, trust model, and trust_level enum.

This document defines HTTP API exposure only.

---

## API Versioning

All endpoints use the `/v1` prefix:

```http
POST /v1/events
GET  /v1/proof/{event_id}
GET  /v1/verify/{event_id}
GET  /v1/account/capabilities
GET  /v1/backup/*
```

Breaking changes require a new version prefix (`/v2/`).

The v1 contract is frozen for backward compatibility after implementation.

---

## 1. AUTHENTICATION

All endpoints require:

```http
X-API-KEY: <key>
```

API key resolves:

```text
AuthedAccount → account_id
```

See SYSTEM_CONTRACT §13 and §16.

### Ownership

For:

```http
GET /v1/proof/{event_id}
GET /v1/verify/{event_id}
```

the server MUST verify:

```text
event_id belongs to account_id from X-API-KEY
```

If access is denied:

```http
HTTP 403 FORBIDDEN
```

`403 FORBIDDEN` is used instead of `404` for cross-account access to `event_id`, accepting the minor information-disclosure trade-off (existence of `event_id` is revealed) in favor of debuggability. Revisit if enumeration abuse becomes a concern.

> **TODO (docs drift):** v1 implementation currently returns `404 NOT_FOUND` (`not_found`) for foreign/missing `event_id` on proof/verify (non-leak policy); reconcile §1 with §6/§7 before v1 contract freeze.

---

## 2. ERROR FORMAT

All errors use a single format:

```json
{
  "error": {
    "code": "EVENT_NOT_FOUND",
    "message": "Event does not exist",
    "request_id": "uuid"
  }
}
```

| HTTP | code            | Meaning                                 |
| ---- | --------------- | --------------------------------------- |
| 400  | INVALID_REQUEST | Invalid JSON or missing required fields |
| 401  | UNAUTHORIZED    | Missing or invalid API key              |
| 403  | FORBIDDEN       | Valid key but no access                 |
| 404  | NOT_FOUND       | Resource does not exist                 |
| 409  | CONFLICT        | Idempotency conflict                    |
| 422  | UNPROCESSABLE   | Semantic validation error               |
| 429  | RATE_LIMITED    | Rate limit exceeded                     |
| 500  | INTERNAL_ERROR  | Internal server error                   |

Additional domain-specific codes (e.g. `PROOF_NOT_READY`, `PROOF_GENERATION_FAILED`) are returned with the HTTP status defined in the relevant endpoint section.

---

## 3. ENDPOINTS OVERVIEW

| Method | Path                         | Purpose                          |
| ------ | ---------------------------- | -------------------------------- |
| POST   | `/v1/events`                 | Submit a new event               |
| GET    | `/v1/proof/{event_id}`       | Retrieve proof artifact          |
| GET    | `/v1/verify/{event_id}`      | Verify chain and optional file   |
| GET    | `/v1/account/capabilities`   | Account plan and entitlements    |
| GET    | `/v1/backup/*`               | Placeholder — not v0.1 contract  |

Notes:

- There is exactly one verification endpoint: `GET /v1/verify/{event_id}`.
- `/verify/{chain_id}` is **not** part of this API.
- Verification always applies to a single event proof, not a chain-wide aggregate.

---

## 4. POST /v1/events

Submit a new event to the ledger.

### Request

```json
{
  "chain_id": "...",
  "file_hash": "...",
  "event_type": "commit"
}
```

### Headers

```http
Idempotency-Key: <uuid>
```

`Idempotency-Key` is **OPTIONAL**. If omitted, no deduplication is performed and a new event is always created.

### Idempotency rules

| Condition                                              | Behavior                                      |
| ------------------------------------------------------ | --------------------------------------------- |
| Same `Idempotency-Key` + same request body within 24h  | Return original response; do not create event |
| Same `Idempotency-Key` + different request body        | HTTP 409, code `CONFLICT`                     |

`Idempotency-Key` is scoped per `account_id`. The same key used by different accounts does not collide.

### Response

```json
{
  "event_id": "...",
  "chain_id": "...",
  "sequence": 5,
  "proof_status": "anchored",
  "trust_level": "BASIC"
}
```

### `proof_status` enum

| Value    | Meaning                                            |
| -------- | -------------------------------------------------- |
| pending  | Event accepted, proof not generated                |
| anchored | Proof generated, Merkle root and signature created |
| failed   | Proof generation failed                            |

No other `proof_status` values are valid.

### `trust_level`

Do not define a new enum here. Valid values:

```text
BASIC
ENHANCED
VAULT
IDENTITY
```

Source of truth: SYSTEM_CONTRACT §14 and §15.

---

## 5. DATA FORMATS

### Cryptographic fields

```text
file_hash
merkle_root
signature
public_key
```

- Lowercase hex string
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

## 6. GET /v1/proof/{event_id}

Returns the primary proof artifact for an event.

### Snapshot semantics

Proof reflects the **chain state at the moment this event was committed**, not the
current chain head:

- **Merkle root** — computed from events with `sequence <=` this event's `sequence`
  (prefix ending at the target event).
- **Signature message** — `chain_id:merkle_root:chain_head` where `chain_head` is
  **this event's** `event_id`, not the latest event in the chain.

Later events appended to the chain do not change proof returned for earlier events.
The server may recompute proof on read; Ed25519 signatures are deterministic for
fixed inputs.

### Response when proof is ready

```json
{
  "proof_version": "proof_v1",
  "proof_type": "merkle-root-v1",
  "leaf_version": "leaf_v1",
  "event_id": "",
  "chain_id": "",
  "sequence": 0,
  "parent_event_id": "",
  "file_hash": "",
  "merkle_root": "",
  "signature": "",
  "public_key": "",
  "tsa": null,
  "created_at": "",
  "proof_status": "anchored",
  "request_id": ""
}
```

### Response when proof material is not yet available

HTTP **200 OK** (the event exists — do not use `404`):

```json
{
  "event_id": "",
  "chain_id": "",
  "sequence": 0,
  "proof_status": "pending",
  "request_id": ""
}
```

`tsa` is `null` when TSA state does not exist. The API MUST NOT fabricate TSA information.

---

## 7. GET /v1/verify/{event_id}

Verifies cryptographic proof and chain structure for a **single event**, optionally
comparing a submitted file hash to the event's stored hash.

Verification uses the **same historical snapshot semantics as [§6 GET /proof](#6-get-v1proofevent_id)**:

- Merkle root and signature are computed from the prefix `sequence <=` this event's
  `sequence`, not from the current chain head.
- `chain_head` in the signature message is **this event's** `event_id`.
- Later events appended to the chain MUST NOT change verification results for
  earlier events.

Implementations MUST reuse the prefix definition in §6; verify MUST NOT reintroduce
full-chain Merkle recomputation for historical `event_id` values.

### Ownership

Same rule as §6: the server verifies `event_id` belongs to the authenticated
`account_id` **before** loading proof material or checking `proof_status`.

### File verification input

Use query parameter (hash only — no file upload in v1):

```http
GET /v1/verify/{event_id}?file_hash=<hex>
```

| Condition              | Result                                      |
| ---------------------- | ------------------------------------------- |
| `file_hash` provided   | `file.status` calculated (`VALID` / `TAMPERED`) |
| `file_hash` omitted    | `file.status` = `NOT_PERFORMED` (not an error) |

Omitting `file_hash` is expected behavior: chain verification still runs when
proof is anchored; file comparison is simply skipped.

### Response shape (anchored proof)

The response exposes **two independent objects** — `chain` and `file`. There is
**no** top-level combined `valid` field.

```json
{
  "event_id": "",
  "chain_id": "",
  "sequence": 0,
  "proof_status": "anchored",
  "chain": {
    "valid": true,
    "merkle_valid": true,
    "signature_valid": true,
    "errors": []
  },
  "file": {
    "status": "NOT_PERFORMED"
  },
  "tsa": null,
  "request_id": ""
}
```

With `?file_hash=<hex>` when hash matches:

```json
{
  "event_id": "",
  "chain_id": "",
  "sequence": 0,
  "proof_status": "anchored",
  "chain": {
    "valid": true,
    "merkle_valid": true,
    "signature_valid": true,
    "errors": []
  },
  "file": {
    "status": "VALID"
  },
  "tsa": null,
  "request_id": ""
}
```

### TSA

`tsa` follows the same rule as §6: `null` when no TSA token exists for the
**prefix Merkle root** used in verification. The API MUST NOT fabricate TSA
information.

When present, `tsa` uses the same object shape as §6 (`timestamp`, `serial`,
`token_bytes`).

### Chain vs file verification

Chain verification and file verification are independent.

The API MUST NOT expose a combined validity state at the top level.

- `chain.valid` MUST NOT imply file verification.
- `file.status` MUST NOT affect `chain.valid`.
- A response with `chain.valid=true` and `file.status=NOT_PERFORMED` is valid.

See SYSTEM_CONTRACT §4.1 and §4.2.

### `file.status` enum

| Value         | Meaning                           |
| ------------- | --------------------------------- |
| NOT_PERFORMED | No file comparison executed       |
| VALID         | Submitted hash matches event hash |
| TAMPERED      | Submitted hash differs            |

No other `file.status` values are valid.

### Pending / failed proof behavior

Applied **after** ownership check, **before** returning a verification body:

| `proof_status` | HTTP | `error.code`              |
| -------------- | ---- | ------------------------- |
| anchored       | 200  | — (normal verification)   |
| pending        | 409  | `PROOF_NOT_READY`         |
| failed         | 422  | `PROOF_GENERATION_FAILED` |

When proof is not ready, the server MUST NOT return `chain.valid=true` or any
verification body implying success.

Example (`proof_status: pending`):

```json
{
  "error": {
    "code": "PROOF_NOT_READY",
    "message": "Proof material is not yet available for this event",
    "request_id": ""
  }
}
```

`/verify` MUST NOT return `chain.valid=true` when `proof_status != anchored`.

---

## 8. GET /v1/account/capabilities

See SYSTEM_CONTRACT §13.

This document does not duplicate capability fields or entitlement rules.

---

## 9. GET /v1/backup/*

This section is a placeholder.

Wildcard paths are not a valid API contract.

Concrete backup endpoints are **OUT OF SCOPE** for v0.1-draft.

They will be specified in a future revision.

---

## 10. OPEN QUESTIONS

### File verification input

To be decided:

- Raw file upload
- Hash only

Recommendation for v1:

```text
v1 uses hash only.
```

### Verification rate limiting

Should `/verify/{event_id}` be rate-limited per API-key or per IP independent of commit quota?
