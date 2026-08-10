# Stage 1 Audit — Evidence Record Layer Foundation

**Date:** 2026-08-10  
**Scope:** Analysis of the current data / integrity / crypto model, storage choice for Evidence Record, and future `evidence_package.zip` contract.  
**Constraint:** Evidence Record is a **projection layer**, not a new source of truth.

---

## Architectural principle

```
Ledger Event  →  Existing Proof Data  →  Evidence Record (projection)
```

Authoritative sources remain: `events` (Postgres), `proof_v1` JSON, TSA tokens, identity keys.  
Evidence Record aggregates pointers + presentation metadata for search, export, certificates, and verifiers.

---

## 1. Where file information lives today

| Field | Server DB (`events`) | Proof JSON (`proof_v1`) | Local (GUI / CLI) |
| --- | --- | --- | --- |
| filename | Absent | Absent | GUI `file_name` / path basename; `proofs/local_copies.json` maps `event_id → path` |
| size | Absent | Absent | GUI in-memory; not persisted on server |
| mime | Absent | Absent | Absent |
| sha256 (`file_hash`) | `events.file_hash` | `events[].file_hash` | Computed at commit |
| event_id | PK | leaf + filename | proof path |
| chain_id | FK | top-level | `project.json` |
| timestamp | `events.created_at` | optional via TSA / API `created_at` | — |
| proof / signature | `events.signature` | nested `proof.signature` | `~/.evident/proofs/{chain_id}/{event_id}.json` |

### Concrete locations

| Artifact | Path / structure |
| --- | --- |
| Event row | `migrations/20260628192818_init.sql` — `events(event_id, chain_id, parent_event_id, file_hash, signature, …)` |
| Request model | `src/models/event.rs` — `file_hash` only (no filename/mime/size) |
| On-disk proof | `src/client.rs` → `~/.evident/proofs/{chain_id}/{event_id}.json` (`ProofFile`) |
| Proof schema | `docs/proof_v1.schema.md` (frozen) |
| Public registry | `public_proof_registry` — hash-keyed public projection (`pv_…`), not full file metadata |
| GUI copies | `{project}/proofs/local_copies.json` |

### Combined object today?

**No.** Closest aggregates:

- `ProofFile` / `export_proof` — chain + leaves (hash) + merkle root signature + optional TSA; **no** filename/size/mime.
- v1 anchored proof — single event crypto snapshot; **no** file metadata.
- Hash attestation — multi-chain matches for one hash; not a registration record.

Data is **split** across event DB, proof JSON, public registry, and local GUI paths.

---

## 2. Hash re-registration policy (as implemented)

| Question | Finding |
| --- | --- |
| SHA-256 as PRIMARY KEY? | **No** for ledger events. Indexes only: `idx_events_file_hash`, `idx_events_file_hash_chain`. |
| Unique constraints | `(chain_id, idempotency_key)`, `(chain_id, sequence)` — not content hash. |
| Lookup before insert? | **No** on submit. Duplicate `file_hash` is allowed. |
| Public registry | At most one **enabled** public proof per `file_hash` (first materialization wins). |

### Scenarios

| Scenario | Outcome |
| --- | --- |
| Same file, same hash, new registration (new idempotency key), same chain | **New event** (new `event_id`, next sequence). |
| Same request + same idempotency key | Idempotent replay. |
| Same file / hash, two projects (two chains) | **Two independent events**. |
| Same hash, public materialization | Later anchors do not replace the first enabled public id. |

**Stage 1 decision (aligned with current ledger):** one SHA-256 **may** have multiple Evidence Records (e.g. Project A + Project B). SHA-256 is **not** a primary key of Evidence Record.

---

## 3. Integrity model (actual)

**Parent-linked event chain + Merkle root over full leaf set.**

Not a classic content `prev_hash` chain. Not a Merkle **inclusion path** (no siblings / positions).

```
event_n.parent_event_id → event_{n-1}.event_id
leaf = SHA256(sequence || event_id || parent_event_id || file_hash)
root = merkle-root-v1(leaves)
signature = Ed25519(chain_id:root:chain_head)
```

| Piece | Location |
| --- | --- |
| Parent / sequence checks | `src/service/verification.rs` — `check_event_structure` |
| Leaf + root | `src/merkle.rs` |
| Proof type | `merkle-root-v1` (`src/proof_format.rs`) |
| Offline verify | Recomputes root from **full** `events[]` (`src/bin/verify.rs`) |

**Stage 1 implication:** do **not** invent `siblings[]` / inclusion proofs. Independent verification for an Evidence Record is:

```
Evidence → ProofFile / event list → parent-chain + merkle-root recompute → Ed25519 verify
```

Documented as: `verification = hash chain validation + merkle-root-v1 (full leaves)`.

---

## 4. Cryptography (actual)

| Item | Value |
| --- | --- |
| Algorithm | **Ed25519** (`ed25519-dalek`, `src/signing.rs`) |
| Signed message | `"{chain_id}:{merkle_root}:{chain_head}"` |
| Signature size | 64 bytes → 128 hex chars |
| Public key | 32 bytes → 64 hex chars in `proof.public_key` |
| Private key | `SIGNING_KEY_PATH` (raw 32-byte seed or Base64) |
| Client pin | `~/.evident/server_identity.pub` (TOFU) |

`proof.signature` / `events.signature` are **real Ed25519 signatures**, not hash commitments.  
`file_hash` is a separate SHA-256 content fingerprint.  
HMAC is used only for Paddle webhooks — not evidence.

**Stage 1 labeling:** keep field name `signature` (honest). No rename to `integrity_commitment`.

Optional user identity signatures are also Ed25519 over the canonical leaf hash (`src/service/identity_signing.rs`).

---

## 5. Proof data structure

### Present (`proof_v1` / `ProofFile`)

- `leaf_version`, `chain_id`, `head_event_id`
- `events[]`: `sequence`, `event_id`, `parent_event_id`, `file_hash`
- `proof`: `root`, `chain_head`, `signature`, `public_key`, `leaves_count`, `version`, `type`
- optional `tsa`: `timestamp`, `serial`, `token_bytes` (+ verification status on read path)

### Absent from `proof_v1`

- filename, size, mime
- evidence_id / certificate_id / lifecycle
- Merkle inclusion path (`siblings`, `positions`)
- local file availability

`proof_v1` is **frozen** (`docs/proof_v1.schema.md`). Extending it would require `proof_v2`.

---

## Storage choice for Evidence Record

### Decision: **Variant A — `evidence/{evidence_id}.json`**

| Criterion | A: sidecar JSON | B: extend proof |
| --- | --- | --- |
| Source-of-truth separation | Projection beside proof | Mixes presentation into crypto artifact |
| `proof_v1` immutability | Preserved | Forces `proof_v2` |
| Multiple evidence per hash | Natural (one file per registration) | Awkward |
| Matches existing layout | Same pattern as `proofs/{event_id}.json` | Overloads verify tooling |

**Local root:** `~/.evident/evidence/{evidence_id}.json` (mirrors `~/.evident/proofs/…`).  
**Not** a new Postgres database. Events / proofs remain authoritative; Evidence Record may be rebuilt from them + optional client metadata.

---

## Lifecycle model (Stage 1)

```
CREATED → REGISTERED → TSA_CONFIRMED → CERTIFIED
                              ↘ REVOKED (reserved; logic not implemented)
```

| Status | Meaning |
| --- | --- |
| `CREATED` | Local preparation only (optional pre-commit). |
| `REGISTERED` | Hash computed, event accepted by ledger. |
| `TSA_CONFIRMED` | TSA token present and verified. |
| `CERTIFIED` | Ledger structure + Ed25519 + TSA verification all succeed. |
| `REVOKED` | Field reserved; full revoke flow **not** implemented. |

**`EXPIRED`:** not in the active enum. Reserved only as a future extension; no TTL / key / TSA expiry policy in Stage 1.

---

## Identifiers

| ID | Rule |
| --- | --- |
| `evidence_id` | `ev_{event_id}` (simple hex, deterministic per event) |
| `certificate_id` | `cert_{UUIDv5(CERTIFICATE_NAMESPACE, event_id bytes)}` — one event → one stable certificate id |

---

## Task 1.5 — Verification API (no artificial Merkle path)

Because the system signs a **merkle root over the full leaf list** and does not store inclusion paths:

- **Do not** add `verify_merkle_proof(siblings, positions)`.
- **Do** provide `verify_evidence_integrity(record, proof)` that:
  1. Locates the event leaf by `event_id` / `file_hash`;
  2. Validates parent chain + sequence;
  3. Recomputes merkle root;
  4. Verifies Ed25519 over `chain_id:root:chain_head`.

---

## 1.7 Future `evidence_package.zip` contract (ZIP not built in Stage 1)

```
evidence_package.zip
├── manifest.json
├── file_hash.json
├── event.json
├── chain.json
├── merkle.json          # full-leaf merkle-root-v1 snapshot (not inclusion path)
├── signature.json
├── tsa.tsr
├── certificate.json
└── README.txt
```

| Package file | Source (projection / SoT) |
| --- | --- |
| `manifest.json` | Evidence Record + package version / created_at |
| `file_hash.json` | `events.file_hash` + optional filename/size/mime from Evidence Record |
| `event.json` | Event row / leaf (`event_id`, `parent_event_id`, `sequence`, `file_hash`, `created_at`) |
| `chain.json` | `chain_id`, head, sequence bounds from chain / proof |
| `merkle.json` / `hashchain.json` | `ProofFile.proof` + `events[]` (root + full leaves; parent links = hash-chain aspect) |
| `signature.json` | `proof.signature`, `proof.public_key`, signed message components |
| `tsa.tsr` | `tsa_tokens.token` / `ProofFile.tsa.token_bytes` (RFC 3161) |
| `certificate.json` | Evidence Record (`certificate_id`, lifecycle, links) |
| `README.txt` | Static human instructions (future Stage) |

---

## Mapping to future products

| Future product | Stage 1 foundation |
| --- | --- |
| File Certificate | Evidence Record + `certificate_id` + file metadata fields |
| Event Certificate | `event_id` link + proof leaf |
| Chain Certificate | `chain_id` + full-leaf merkle snapshot (existing SAC direction) |
| Public Verifier | Hash lookup → `find_evidence_by_hash` + public registry (separate layer) |
| `evidence_package.zip` | Table above; generation deferred |

---

## Acceptance checklist (Stage 1)

1. Evidence Record exists after commit (file meta + hash + event + chain + status + `certificate_id`).
2. Lifecycle reflects REGISTERED / TSA_CONFIRMED / CERTIFIED from real proof/TSA state.
3. Same hash → multiple evidence records allowed; `find_evidence_by_hash` returns a list.
4. Crypto model documented: parent chain + merkle-root-v1 + Ed25519 + TSA.
5. Package table maps future ZIP contents to existing sources without inventing Merkle inclusion proofs.
