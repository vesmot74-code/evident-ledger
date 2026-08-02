# proof_v1 / TSA Alignment Audit

Date: 2026-08-02

Branch: `stage-13.7-release-candidate`

## Decision

**Option A (confirmed):** TSA is an **external attestation layer**, not part of
the immutable `proof_v1` core schema.

`docs/proof_v1.schema.md` remains frozen and is **not** modified.

---

## Starting hypothesis

TSA is a separate attestation layer sealed *after* the core proof digest exists:

```text
core proof (proof_v1)
    ↓
sealed merkle root + server signature
    ↓
RFC3161 timestamp attestation (external)
```

RFC3161 stamps an existing digest; it does not participate in constructing the
proof signature payload (`chain_id:merkle_root:chain_head`).

---

## Runtime evidence

### Where `proof_v1` is formed

| Location | Role |
|----------|------|
| `src/proof_format.rs` | `PROOF_VERSION = "proof_v1"` |
| `src/service/verification.rs` → `export_proof` | Builds JSON with nested `proof: { version: "proof_v1", … }` and sibling `tsa` |
| `src/client.rs` → `ProofFile` / `ProofPayload` | On-disk CLI artifact: `proof` object + optional top-level `tsa` |
| `src/signing.rs` → `sign_root` | Signature covers `chain_id:merkle_root:chain_head` only |

Canonical export shape (`export_proof`):

```json
{
  "leaf_version": "leaf_v1",
  "chain_id": "…",
  "head_event_id": "…",
  "events": [ … ],
  "proof": {
    "version": "proof_v1",
    "type": "merkle-root-v1",
    "root": "…",
    "leaves_count": N,
    "chain_head": "…",
    "signature": "…",
    "public_key": "…"
  },
  "tsa": {
    "timestamp": …,
    "serial": "…",
    "token_bytes": …
  }
}
```

`tsa` is a **sibling** of `proof`, never nested inside the `proof_v1` object.

### Where TSA data lives

| Location | Storage / fields |
|----------|------------------|
| DB table `tsa_tokens` | `tsa_token`, `tsa_timestamp`, `tsa_serial`, `verification_status`, … |
| Write path | `src/tsa_worker.rs` inserts into `tsa_tokens` after anchoring |
| Read / API | `src/api/v1/proof_state.rs` builds response `tsa{…}` from DB + verify |
| CLI `ProofFile` | Optional `tsa: Option<TsaData>` beside `proof: ProofPayload` |

Grep hits for `tsa_timestamp` / `tsa_token` / `tsa_signature` in runtime are
overwhelmingly **DB columns, API `tsa` objects, or workers** — not fields of
the nested `proof` object with `version: "proof_v1"`.

### Signature invariant (unchanged)

From `docs/proof_v1.schema.md` and `ServerSigner::sign_root`:

```text
signature MUST cover (root + chain_id + head_event_id)
```

No TSA field participates in the signed message.

### Legacy / non-authoritative structures

`src/freeze.rs` defines a separate experimental `Proof` struct that embeds
`tsa_timestamp` / `tsa_signature` / `verification_status`. That type is **not**
the production `proof_v1` on-disk/API contract used by `export_proof` /
`ProofFile` / `evident-verify`. It must not be treated as the frozen schema.

---

## Contract model (Option A)

```text
proof_v1
 |
 + merkle root
 + chain head
 + server signature
 + (optional) identity signature extension elsewhere
 + events[] / leaf material

tsa_attestation  (external layer)
 |
 + timestamp
 + token / serial / token_bytes
 + verification_status (+ optional verification_reason)
 + certificate / trust material (verify path)
```

Rule:

> TSA attestation is not part of proof_v1 core schema.
> It is an external cryptographic timestamp layer.

---

## Documentation drift addressed

| Document | Issue | Resolution |
|----------|-------|------------|
| `docs/proof_v1.schema.md` | Correct frozen core; no TSA | **Unchanged** |
| `docs/protocol_v0.1.md` | “Canonical proof artifact” listed TSA fields inline | Updated to Option A boundary |
| `docs/VERIFY_MODEL.md` | Already treats TSA as layer / API gate | Clarifying rule added |
| `docs/API.md` | Already returns sibling `tsa` on proof responses | Clarifying rule added |

Forbidden state avoided: claiming `proof_v1` is frozen **and** contains TSA
fields without a version bump.

---

## Regression

`tests/proof_schema_contract.rs` locks:

- core `proof_v1` field set;
- TSA fields absent from the nested `proof` object;
- presence of optional top-level / sibling `tsa` allowed;
- comment that schema changes require `proof_v2`.
