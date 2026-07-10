# SYSTEM_CONTRACT.md — Evident Ledger (Current State)

Deterministic verifiable event ledger with cryptographic proofs and offline verification.

**Status:** This describes the actual implementation as of July 2026, not the idealized architecture.

---

## 1. SYSTEM MODEL

- Every action is an immutable event.
- Events form a hash-linked chain.
- Trust is established through cryptographic proof (Merkle root + cryptographic signature verification).
- Offline verification is possible via `evident-verify`.

---

## 2. STORAGE MODEL

The system currently utilizes two independent storage backends.

## 2.1 GUI Storage (`evident-gui`)



```text
~/Evident Projects/<project_name>/
  originals/
  proofs/
  Audit/
    audit.jsonl
```

Used by the `evident-gui` application.

Each project contains:

```text
project.json
```

with a unique `chain_id` (UUID v4).

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
```

The cryptographic pipeline is identical for GUI and CLI.

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

---

## 4.2 Local File Verification

The GUI additionally verifies local files.

Verification compares:

```text
originals/*
```

SHA-256 hashes against recorded:

```text
file_hash
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

# 6. ORIGINAL FILE NAMING

Format:

```text
originals/{sequence:04}_{filename}
```

Examples:

```text
0001_document.rtf
0002_report.pdf
```

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
* GUI ZIP export button exists but functionality is not implemented.
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

Verified manually through regression testing:

* original file modification detection
* hash mismatch detection

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
* CLI verification workflow

Future improvements:

* unified storage model
* automated ZIP evidence export
* expanded automated test coverage
* additional TSA providers
