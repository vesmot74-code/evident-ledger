Вот **финальная, согласованная с CLI и FREEZE v0.1 версия README**. Она выровнена под твой текущий кодовый контур: `commit → verify → status → report generate`.

Можно вставлять без изменений.

---

# 📄 README.md (EVIDENT LEDGER v0.1 FROZEN)

````markdown
# Evident Ledger (v0.1 FROZEN)

Deterministic verifiable event ledger with cryptographic proofs and offline verification.

---

## 🧠 SYSTEM OVERVIEW

Evident Ledger is a cryptographic event system where:

- every action is an immutable event
- events form a hash-linked chain
- trust is derived from cryptographic proof, not server state
- all results are reproducible offline

---

## 🔧 CORE PIPELINE

```text
file → SHA256 → event → chain → proof → verify → report
````

---

## 🚀 QUICK START

### Build project

```bash
cargo build --bin evident
```

---

### Initialize local identity

```bash
evident init
```

---

### Create new chain

```bash
evident new-chain
```

---

### Commit file into chain

```bash
evident commit <file> --chain <chain_id>
```

Example:

```bash
evident commit Cargo.toml --chain 11111111-1111-1111-1111-111111111111
```

---

### Verify proof (offline)

```bash
evident verify ~/.evident/proofs/<chain_id>/*.json
```

Output:

```text
OK: proof valid
```

---

### Generate PDF report

```bash
evident report generate <chain_id>
```

Output:

```text
~/.evident/proofs/<chain_id>/
  ├── proof.json
  └── proof.pdf
```

---

### Check chain status

```bash
evident status <chain_id>
```

---

## 📦 COMMANDS CONTRACT

| Command         | Description                                         |
| --------------- | --------------------------------------------------- |
| init            | Initialize local cryptographic identity             |
| new-chain       | Create new ledger chain                             |
| commit          | Append immutable event to chain                     |
| verify          | Offline verification of proof                       |
| status          | Show chain state                                    |
| report generate | Generate deterministic proof artifacts (JSON + PDF) |

---

## 🧱 ARCHITECTURE

```text
Ledger Engine   → immutable event chain
Verifier        → offline cryptographic validation
TSA Layer       → timestamp attestation authority
Report Engine   → deterministic proof exporter
CLI             → orchestration layer (no business logic)
```

---

## 🔐 CORE RULES (FREEZE v0.1)

### 1. Append-only system

Events are immutable and cannot be modified or deleted.

---

### 2. Deterministic hashing

All hashes use SHA-256 only.

---

### 3. Chain integrity

Each event references the previous event hash.

```text
event[i].parent_hash = hash(event[i-1])
```

---

### 4. Offline verification

Verification must work without server access.

---

### 5. Server is not truth

Truth is derived from cryptographic proof, not server state.

---

## 📄 PROOF MODEL

Proof is the canonical output of the system:

```text
Proof = {
  chain_id,
  root_hash,
  tsa_timestamp,
  tsa_signature,
  event_count,
  verification_status
}
```

---

## 📊 OUTPUT GUARANTEE

Given identical input:

* proof.json is identical
* proof.pdf is byte-identical
* verification result is identical

Determinism is a hard requirement of the system.

---

## 🧪 TESTS

```bash
cargo test --lib
```

---

## ⚠️ FREEZE RULE (CRITICAL)

This is protocol version v0.1.

### Hard constraints:

* NO breaking changes allowed
* NO schema modifications
* NO CLI contract changes without version bump
* NO silent behavior changes

### If change is required:

→ must introduce v0.2 protocol

---

## 📌 STATUS

* CLI: stable
* Ledger: stable
* Verifier: stable
* Report engine: integrated
* Protocol: FROZEN v0.1

---

## 🧠 FINAL NOTE

This system defines a deterministic model of verifiable events.

The source of truth is not the server — it is the cryptographically validated event chain.

```

