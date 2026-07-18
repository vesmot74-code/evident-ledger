# SYSTEM_CONTRACT.md — Evident Ledger (Current State)

Deterministic verifiable event ledger with cryptographic proofs and offline verification.

**Status:** This describes the actual implementation as of July 2026, not the idealized architecture.

Version: 1.0-draft
Status: Active Development
Last Updated: 2026-07-16

---

## 1. SYSTEM MODEL

- Every action is an immutable event.
- Events form a hash-linked chain.
- Trust is established through cryptographic proof (Merkle root + cryptographic signature verification).
- Offline verification is possible via `evident-verify`.

---

## 2. STORAGE MODEL

Evident Ledger follows a zero-file custody model.
Original documents are never stored automatically.
The system stores cryptographic references only.
Optional local copies are user-controlled and referenced through `local_copies.json`.

The system currently utilizes two independent storage backends.

## 2.1 GUI Storage (`evident-gui`)

```text
~/Evident Projects/<project_name>/
  project.json
  proofs/
    <event_id>.json
    local_copies.json   (optional, user-controlled)
  audit/
    audit.jsonl
```

Used by the `evident-gui` application.

Each project contains:

```text
project.json
```

with a unique `chain_id` (UUID v4).

The GUI never creates an `originals/` directory and never copies user files automatically.
After commit, the user may optionally save a local copy; if they do, the absolute path is recorded in `proofs/local_copies.json`.

---

## 2.2 CLI Storage (`evident`)

```text
~/.evident/
  identity.key
  identity.pub
  events.jsonl
  proofs/
    <chain_id>/
```

Used by CLI commands:

```text
evident init
evident commit
evident report generate
evident status
```

---

## 2.3 Storage Synchronization Status

GUI and CLI storage systems are currently separate.

A file committed through GUI storage is not automatically visible to CLI commands.

A file committed through CLI storage is not automatically visible to GUI.

This separation is a known technical limitation resulting from parallel development paths.

Future work includes migration into a unified storage model.

---

# 3. CORE PIPELINE

```text
file
 ↓
SHA256
 ↓
event
 ↓
chain
 ↓
proof
 ↓
verify
 ↓
report
 ↓
(optional) user saves local copy → local_copies.json
```

The cryptographic pipeline is identical for GUI and CLI.
The GUI never stores the original file automatically.
Storage entry points differ.

---

# 4. VERIFICATION MODEL

## 4.1 Backend Verification

Endpoint:

```http
GET /verify/{chain_id}
```

The verification service performs:

* event chain validation
* Merkle root recomputation
* signature verification
* chain integrity verification

Response:

```json
{
  "valid": true,
  "blocks": 0,
  "head_event_id": "",
  "errors": []
}
```

**Scope note:** `valid` reflects chain integrity only — Merkle root,
signature, and event-link consistency. It does NOT indicate anything
about a local file. This endpoint has no knowledge of user-controlled
local copies (see §2 Storage Model). A response of `valid: true` MUST
NOT be interpreted or displayed as "file verified." Clients combining
this result with local file status MUST keep them as two independent
fields, per §4.2.

The API MUST NOT expose a combined validity state that merges backend
verification and local file verification results.

---

## 4.2 Local File Verification

This check is independent of backend verification (§4.1) and MUST
be surfaced as a separate field, never merged into a single `valid`
value.

The GUI optionally verifies user-controlled local copies.

Verification reads paths from:

```text
proofs/local_copies.json
```

and compares SHA-256 hashes against recorded:

```text
file_hash
```

If no local copy was saved by the user, integrity status is:

```text
NOT STORED (local_integrity_ok = None)
```

If a local copy exists and matches:

```text
VALID (local_integrity_ok = Some(true))
```

If a local copy exists but the hash differs:

```text
TAMPERED (local_integrity_ok = Some(false))
```

Final GUI status consists of two independent checks:

```text
backend_valid
```

Cryptographic chain integrity.

and:

```text
local_integrity_ok
```

Local file integrity.

---

## 4.3 Offline Verification

CLI verifier:

```text
evident-verify
```

uses:

```text
proof.json
```

Verification includes:

* Merkle root validation
* signature validation
* signer identity verification
* optional original file comparison

Possible results:

```text
Original: OK
```

or:

```text
Original: MISSING or MISMATCH
```

---

# 4.4 Merkle Leaf Canonicalization

Merkle proof integrity is based on canonical leaf construction.

Each Merkle leaf includes the complete event identity context:

```text
SHA256(
    sequence ||
    event_id ||
    parent_event_id ||
    file_hash
)
```

The leaf calculation includes:

* event sequence number
* event UUID
* parent event UUID
* committed file SHA-256 hash

This prevents undetected modification of event identity fields.

Any change to:

```text
sequence
event_id
parent_event_id
file_hash
```

changes the Merkle leaf and results in Merkle root mismatch during verification.

Offline verification rejects proofs where:

```text
recomputed_merkle_root != signed_merkle_root
```

This guarantees that event identity and chain structure are cryptographically bound together.

## Version fields (required)

Every supported proof artifact MUST declare:

```text
proof.version: "proof_v1"
proof.type:    "merkle-root-v1"
leaf_version:  "leaf_v1"
```

### Two versioning axes

- `proof.type` — top-level proof mechanism type (for example `merkle-root-v1`). A change to proof structure or proof envelope requires bumping `proof.version` and/or `proof.type`.
- `leaf_version` — Merkle leaf canonicalization version. Current sole supported value `leaf_v1` maps to the formula above (`sequence + event_id + parent_event_id + file_hash`). A change to the leaf formula requires bumping only `leaf_version`.

`proof.type` and `leaf_version` are independent versioning axes.

Proofs without `proof.version` and `leaf_version` are **unversioned legacy** and MUST be rejected by offline verification with:

```text
unversioned legacy proof format — unsupported, please regenerate
```

Proofs with a missing or unsupported `proof.version` or `leaf_version` MUST be rejected with:

```text
unsupported proof format
```

Any future change to the leaf formula MUST bump `leaf_version` (for example `leaf_v2`). Backward-compatible verify for prior leaf versions is not required.

Verified tampering scenarios:

```text
modified event_id        → FAIL
modified file_hash       → FAIL
modified parent_event_id → FAIL
modified signature       → FAIL
```

Valid proofs continue to verify successfully after regeneration with the updated canonical Merkle model.

---

# 5. AUDIT MODEL

GUI audit storage:

```text
Audit/audit.jsonl
```

Append-only event log.

Example record:

```json
{
  "event_id": "...",
  "chain_id": "...",
  "file_hash": "...",
  "sequence": 1,
  "parent_event_id": "...",
  "created_at": "...",
  "kind": {
    "Anchored": {
      "server_event_id": "...",
      "proof": {}
    }
  }
}
```

Each file commitment creates two states:

```text
Submitted
```

Created before server confirmation.

and:

```text
Anchored
```

Created after backend confirmation.

---

# 6. LOCAL COPY REFERENCE MODEL

Optional user-controlled local copies are referenced in:

```text
proofs/local_copies.json
```

Format:

```json
{
  "<event_uuid>": "/absolute/path/to/file.pdf"
}
```

The system never writes to this file automatically during commit.
The user explicitly chooses whether to save a local copy after commit.

---

# 7. TSA (RFC 3161)

The system integrates RFC 3161 timestamping through the notary-tsa layer with external TSA provider support.

TSA information is optional.

If TSA fields are incomplete:

```text
timestamp
serial
token_bytes
```

the generated certificate displays:

```text
TSA Status: Not Verified
```

The system never reports false TSA verification.

---

# 8. REPORT GENERATION

Command:

```bash
evident report generate
```

Requires:

```json
file_hash
chain_id
```

inside:

```text
proof.json
```

If required fields are missing:

```text
incomplete proof: missing <field>
```

PDF generation stops.

TSA fields remain optional.

---

# 9. DEPENDENCIES

Vendored crates:

```text
vendor/notary-tsa
vendor/notary-pdf
```

are included for build self-sufficiency.

Server requirements:

```text
PostgreSQL
```

Database is required for:

```text
evident-ledger
```

server execution.

Build does not require active database connection.

SQLx offline cache:

```text
.sqlx/
```

is used.

---

# 10. KNOWN LIMITATIONS

* GUI and CLI use separate storage backends.
* TSA depends on external provider availability.
* Server requires PostgreSQL.
* Storage unification is planned future work.

---

# 11. VERIFIED FUNCTIONALITY

The following components are covered by tests:

## Audit Chain

```text
audit.jsonl
```

append-only mechanism.

Verified by:

```bash
cargo test
```

---

## Cryptographic Verification

Verified:

* Merkle root calculation
* signature validation
* tamper detection

Tests:

```text
tests/verifier.rs
```

---

## Sequence Validation

Verified:

```text
verify_project
```

CLI sequence checking.

---

## Local Integrity Verification

Verified through `local_copies.json` workflow:

* missing local copy → NOT STORED
* matching hash → VALID
* modified file → TAMPERED

---

# 12. CURRENT IMPLEMENTATION STATUS

Implemented:

* SHA-256 file hashing
* append-only audit chain
* Merkle tree verification
* Ed25519 signatures
* offline verification
* PDF evidence reports
* RFC3161 TSA integration layer
* GUI verification workflow
* GUI ZIP evidence export (via local_copies.json)
* CLI verification workflow

Future improvements:

* unified storage model
* expanded automated test coverage
* additional TSA providers

---

# 13. ACCOUNT CAPABILITY MODEL

The system exposes account-level capabilities.

CLI command:

```bash
evident account
```

Backend endpoint:

```http
GET /account/capabilities
```

Authentication:

```http
X-API-KEY
```

---

# 14. TRUST LEVEL MODEL

Evidence trust level depends on enabled capabilities.

## Basic

Available in FREE plan.

Includes:

- SHA-256 content hash
- Merkle proof
- cryptographic signature
- Machine TSA timestamp

## Enhanced

Available in Legal plan.

Includes:

- Qualified TSA provider

## Vault

Available in Vault plan.

Includes:

- encrypted server backup

## Identity

Available in Identity plan.

Includes:

- user identity binding

---

# 15. COMMIT RESULT OUTPUT

CLI displays trust level, active plan and available upgrades after successful commit.

`trust_level` values are defined exclusively in §14 (Trust Level
Model). Any system output field named `trust_level` (API, CLI, GUI)
MUST use one of:

BASIC
ENHANCED
VAULT
IDENTITY

No other values are valid. This section is the single source of truth
for this enum.

Note: §14 uses title-case section headers (Basic, Enhanced, Vault,
Identity) as prose labels for plan tiers — these are not the enum
values. The enum values are exactly the four upper-case strings above.
Do not rename or "harmonize" the casing in §14 — the two are
intentionally different (prose label vs. field value).

---

# 16. API KEY AUTHENTICATION

Server API requests use API key authentication through X-API-KEY header.

---

# 17. ARCHITECTURE & PUBLIC SECURITY BOUNDARY

**Status:** Frozen at Stage 7. Security invariants: [SECURITY.md](SECURITY.md) §2.5. Verification model: [docs/VERIFY_MODEL.md](docs/VERIFY_MODEL.md).

## 18.1 Source of Truth

The **server** is the authoritative source of truth for **anchored evidence** and **account ownership**.

Local artifacts (PDF reports, ZIP exports, offline `proof.json`, GUI/CLI project files) are verifiable **projections** of server-anchored state — not an independent source of truth. See [SECURITY.md](SECURITY.md) Security Invariant 1.

## 18.2 Ownership Model

- Every anchored event belongs to exactly one **account** (`account_id`).
- Authenticated API operations (`X-API-KEY`) resolve the caller's account and enforce **ownership** before returning event-scoped data.
- Private verification (`GET /v1/verify/{event_id}`) requires authentication and confirms the event belongs to the authenticated account before proof, chain, or file checks.
- Cross-account access to private event metadata returns **404** (no existence side-channel via error type where ownership checks apply first).

Public verification **does not** use account context and **must not** reveal which account(s) registered a hash.

## 18.3 Public vs Private API Boundary

| Layer | Authentication | Primary identifier | Disclosure |
|-------|----------------|-------------------|------------|
| **Private** (`/v1/*`, legacy `/events`, etc.) | `X-API-KEY` required | `event_id`, `chain_id` (within account scope) | Owner-grade: proof state, chain prefix integrity, file hash claim comparison |
| **Public** (`/public/verify`, certificate PDF) | None | `file_hash` or opaque `public_proof_id` | Existence-only: registration fact without ownership or chain structure |

Owner-grade evidence (full chain, Merkle proof, signatures, internal audit trail) is **never** exposed through public HTTP endpoints. See [SECURITY.md](SECURITY.md) Invariants 2, 5.

## 18.4 Materialization Model

Private event ledger → **public-safe projection** (materialized at anchor time):

- Internal proofs and events remain in private storage.
- When a proof becomes eligible, a **public-safe row** is written to the public projection (registry) containing only fields approved for external disclosure.
- Public handlers query **only** the public projection — not `events`, internal proof tables, or account tables.
- The concrete table or view name is an implementation detail; the invariant is "dedicated projection without reversible private references" ([SECURITY.md](SECURITY.md) Invariant 6).

`public_proof_id` is assigned at materialization, opaque (`pv_` + base58), and must not encode internal ids (Invariant 9).

## 18.5 Public Disclosure Boundary

Public responses answer: **"Is this hash currently registered in the public projection?"**

They do **not** answer:

- Who registered it
- How many accounts registered it
- Internal chain or event structure
- Whether a hash was previously submitted but is now disabled (beyond `exists: false` for the current query)

**Zero-disclosure fields** — public JSON, PDF, headers, and URLs **must not** include the forbidden set listed in [SECURITY.md](SECURITY.md) §2.5 Invariant 3 (`chain_id`, `event_id`, `merkle_root`, internal signatures, account identifiers, registration cardinality, and equivalent metadata). Do not duplicate the full list here; treat §2.5 as normative.

Cross-account registration of the same hash must produce the **same public disclosure shape** as a single-account registration (Invariant 4).

---

# 18. FUTURE PRODUCT LAYERS

See [ROADMAP.md](ROADMAP.md) for the full split between frozen architecture, evolvable implementation, and product-layer work.

Summary:

- Vault Layer: planned
- Identity Layer: planned
- Billing Layer: planned
