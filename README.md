# Evident Ledger

**Deterministic cryptographic evidence infrastructure for high-stakes business records.**

![Dashboard](docs/assets/dashboard.png)

**Stop trusting your internal logs for audits.** Evident Ledger transforms business documents, AI model artifacts, and engineering records into independently verifiable proof objects.

Designed for **Pharma, AI Development, and Legal Compliance**, it provides an immutable audit trail without the cost and complexity of blockchain.

---

## ⚡️ Quick Start: See it in Action

Don't just take our word for it. See what a professional auditor receives:

* 📄 **[Evidence Snapshot](docs/samples/evidence_snapshot.pdf)**: Complete verification summary showing ledger integrity and cryptographic proof with Merkle root and digital signature
* 🛡️ **[Hash Attestation Certificate](docs/samples/hash-attestation-66d244d59319785d.pdf)**: Independent verification certificate confirming hash presence in the Evident Ledger system with TSA timestamp

---

## 🏗 Why Evident Ledger?

Traditional logging systems are vulnerable: they store evidence inside the system that defines the truth. **Evident Ledger separates the two.**

| Feature | Your Current Logs | Evident Ledger |
| --- | --- | --- |
| **Integrity** | Database-dependent | Mathematically proven |
| **Verification** | Requires internal access | Independent, offline, public |
| **Tamper-proofing** | Mutable | Immutable Event Chain |
| **Cloud Dependency** | Mandatory | **Zero** (Local execution) |

---

## 🛡️ Trust Model

Our system relies on mathematical rigor, not corporate trust:

* **Cryptographic Hashing:** SHA-256 for file fingerprinting
* **Immutable Event Chain:** Hash-linked events with Merkle proofs
* **External Anchoring:** RFC 3161 TSA integration with freetsa.org
* **Offline Verification:** Independent verification of all proofs without server access

---

## 🚀 How it Works

1. **Commit:** Hash your file locally (the file never leaves your machine)
2. **Anchor:** The server assigns a sequence and provides a cryptographically signed timestamp
3. **Verify:** Anyone can verify the proof independently—no server access required

![Verification](docs/assets/verification.png)

---

## 📊 Evidence Verification Example

Our system produces cryptographically verifiable evidence packages:

- **Ledger Integrity:** Validated with Merkle Root: `8658e621dfaed6f55100e487c4d2d9da133268c5f3907d254c73331cbf784090`
- **Digital Signature:** `c2a2b12bc665888c8f633e461cd6c6f85b72fdbf8386a78ab7a87a8ade77814e`
- **External Timestamp:** Confirmed via freetsa.org TSA
- **Offline Verification:** Self-contained proofs verifiable with `evident verify proof.json`

---

## 🛠 Quick Start (CLI)

```bash
# Build the client
cargo build --release

# Initialize your identity
./target/release/evident init

# Protect a document
./target/release/evident commit document.pdf --chain <chain_id>

# Verify independently
./target/release/evident verify ~/.evident/proofs/<chain_id>/proof.json
📂 Resources
Protocol Specification

Case Studies

Security Policy

Evident Ledger: The truth is not stored. The truth is proven.
