```
# Evident Ledger

**Cryptographic Evidence Engine for Business Records.**

Evident Ledger transforms digital files into **independently verifiable events**. It creates an immutable, tamper-proof audit trail that works even when your primary systems are offline. Built for compliance, auditability, and absolute data integrity.

---

## The Problem
Business records are vulnerable to dispute:
* Documents can be replaced or modified.
* Database logs can be altered.
* Timestamps can be questioned.

Standard systems lack a **chain of custody** that an independent auditor can trust.

## The Solution: Evident Ledger
We implement a **"Server-is-not-Truth"** model. The truth exists in the cryptographic proof itself, not in the state of a database.

* **Hash-based evidence:** Every file is anchored by its unique SHA-256 fingerprint.
* **Immutable chain:** Events are linked into a cryptographically secured chain.
* **TSA Timestamping:** RFC 3161 integration links your data to global time sources.
* **Local Verification:** Trust the math, not the cloud. Verify proofs without calling home.
* **Audit-Ready Reports:** Deterministic PDF reports that are byte-identical across environments.

## No Blockchain Philosophy
Evident Ledger provides cryptographic integrity without the complexity, latency, or dependencies of a blockchain. You retain full control over your data and proofs.

---

## Trust Tiers
Every project has different requirements for legal significance:

| Tier | Level | Key Capabilities | Significance |
| :--- | :--- | :--- | :--- |
| **#1** | **Personal Proof** | Local verification, FreeTSA | Technical existence proof |
| **#2** | **Legal Compliance** | Jurisdiction-specific TSA | Supports legally relevant audit processes |
| **#3** | **Immutable Audit** | TSA redundancy, Corporate storage | Protection against data loss |
| **#4** | **Enterprise** | PKI, Personal/Public Keys | Sovereign digital identity |

---

## Architecture
```text
Event → Hash → Immutable Chain → TSA → Proof Object → Audit Report
```

## Quick Start

### 1. Build the CLI

```bash
cargo build --release
```

### 2. Create your identity

```bash
./target/release/evident init
```

### 3. Protect a document

```bash
./target/release/evident commit <file> --chain <chain_id>
```

### 4. Verify independently

```bash
./target/release/evident verify ~/.evident/proofs/<chain_id>/proof.json
```

---

## Documentation

* [Protocol v0.1 Specification](docs/protocol_v0.1.md)
* [Case Studies](docs/case-studies/)
* [Security Policy](SECURITY.md)

---

*Evident Ledger — The truth is in the math.*
```
