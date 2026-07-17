# Security Policy

## Reporting Vulnerabilities

Please report security vulnerabilities through the project's issue tracker.

We prioritize issues related to:

- Cryptographic integrity.
- Proof verification.
- Chain consistency.
- Unauthorized modification of evidence records.

Please do not disclose sensitive security issues publicly before they are reviewed.

---

## Security Model

Evident Ledger is built around a **cryptographic evidence model** where trust is derived from verification, not from a central database.

The system relies on the following assumptions:

### 1. Local Environment Integrity

The client environment is responsible for protecting:

- Private keys.
- Local proof storage.
- Configuration files.
- Access credentials.

A compromised client device may compromise the creation or storage of evidence.

### 2. Hash Security

Evident Ledger uses cryptographic hashing to fingerprint digital records.

The system assumes the continued security properties of modern hash algorithms, including collision resistance.

### 3. External Timestamp Authority (TSA)

Timestamp services provide an independent reference to external time.

- **Tier 1:** Public TSA providers may be used for basic existence proofs. Availability and long-term retention depend on the provider.
- **Tier 2+:** Enterprise deployments may use jurisdiction-specific TSA services with stronger operational and compliance requirements.

Evident Ledger does not operate as a timestamp authority and does not replace external trust providers.

### 4. No Content Exposure

Evident Ledger is designed around privacy-preserving evidence creation.

Original documents are not required to leave the user's environment. The system operates using cryptographic fingerprints and proof objects.

---

## Integrity Guarantees

Evident Ledger provides:

- Independent verification of evidence records.
- Detection of unauthorized modifications.
- Cryptographic linkage between events.
- Reproducible audit reports.

Evident Ledger proves the integrity and existence of recorded data.

It does not certify:

- The truthfulness of document contents.
- The identity of the document author without additional identity infrastructure.
- Legal validity in every jurisdiction.

---

## Responsible Use

Organizations deploying Evident Ledger should maintain appropriate:

- Key management procedures.
- Access controls.
- Backup strategies.
- Compliance processes.

Cryptographic evidence is strongest when combined with proper operational security.

---

## Public Verification Rate Limiting

Rate limiting is IP-based and per-instance (in-memory).
It is a mitigating control against casual abuse and scraping,
not a hard guarantee against distributed or botnet-based probing.

Public endpoints (`GET /public/verify`, `GET /public/verify/:public_proof_id/certificate.pdf`)
use separate fixed-window limits per client identity (hashed IP, optionally User-Agent).
`TRUST_PROXY_HEADERS` defaults to `false`; forwarded headers are ignored unless explicitly enabled.
