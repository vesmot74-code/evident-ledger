# SYSTEM_CONTRACT.md — Evident Ledger v1.2 (FINAL / CONTROLLED DRIFT)

Deterministic verifiable event ledger with cryptographic proofs and offline verification.

---

## 1. SYSTEM MODEL

Evident Ledger is a project-local deterministic event system.

- every action is an immutable event
- events form a hash-linked chain
- trust is derived from cryptographic proof, not server state
- all outputs are reproducible offline
- no global system state is authoritative

---

## 2. STORAGE MODEL (HARD CONTRACT)

```text
Evident Projects/<project_name>/
  originals/
  proofs/
  Аудит/
    audit.jsonl
```

RULES:

- ONLY “Evident Projects” is valid root
- NO ~/.evident as active storage
- NO HOME-based fallback
- all paths must be deterministic

---

## 3. CORE PIPELINE

```text
file → SHA256 → event → chain → proof → verify → report
```

- pipeline is deterministic
- no external state allowed

---

## 4. LAYERS

CLI:

- orchestration only
- no business logic

GUI:

- user interaction layer
- must not affect ledger semantics

VERIFIER:

- standalone binary
- offline only
- no runtime dependencies

---

## 5. ORIGINALS CONTRACT

Format:

```text
{:04}_<filename>
```

RULES:

- sequence prefix mandatory
- monotonic per project
- immutable after creation
- no overwrite
- no hash-based naming

---

## 6. AUDIT CONTRACT

Path:

```text
Evident Projects/<project>/Аудит/audit.jsonl
```

RULES:

- append-only (append=true)
- no truncate
- no rewrite
- 1 line = 1 JSON event

---

## 7. VERIFY CONTRACT

```bash
evident-verify <proof_path> <original_file_path>
```

OUTPUT:

```text
OK: proof valid
```

RULES:

- fully offline
- deterministic
- no server dependency

---

## 8. PROOF MODEL

Required fields:

- chain_id
- root_hash
- tsa_timestamp
- tsa_signature
- event_count
- verification_status

---

## 9. DETERMINISM RULE

Same input MUST produce:

- identical proof.json
- identical proof.pdf
- identical verification result

---

## 10. FORBIDDEN

- HOME fallback paths
- ~/.evident writes
- mutation of past events
- audit truncation
- nondeterministic timestamps in core logic
- server-truth dependency

---

## 11. CONTROLLED DRIFT

ALLOWED:

- CLI internal refactors
- GUI changes
- verifier optimizations

NOT ALLOWED:

- storage model changes
- audit format changes
- originals naming changes
- verification logic changes

---

## 12. ARCHITECTURE

Ledger Engine → events  
Verifier → validation  
TSA → timestamping  
Report Engine → export  
CLI → orchestration  
GUI → interaction  

---

## 13. FREEZE

Protocol: FROZEN v1.2  
Storage: LOCKED  
Audit: LOCKED  
Verifier: LOCKED  
CLI/GUI: drift allowed only in execution layer
```
