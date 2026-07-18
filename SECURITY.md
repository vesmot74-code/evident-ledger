# Security Policy

## Reporting Vulnerabilities

Please report security vulnerabilities through the project's issue tracker.

We prioritize issues related to:

- Cryptographic integrity
- Proof verification
- Chain integrity and immutability
- Unauthorized modification of evidence records
- Public verification disclosure boundaries
- Abuse of public verification endpoints

Please do not disclose sensitive security issues publicly before they are reviewed.

---

## 2.1 Threat Model

### Adversaries

| Actor | Capability | Typical goal |
|-------|------------|--------------|
| **Unauthenticated observer** | HTTP access to public endpoints only | Learn whether specific hashes are registered; infer relationships or account structure |
| **Malicious client** | High-volume automated requests from one or many IPs | Mass hash oracle (leaked hash lists, brute-force probing); scrape registration facts |
| **Compromised or misconfigured reverse proxy** | Can inject or strip `X-Forwarded-For` / `X-Real-IP` if trusted | Shift rate-limit identity; hide true origin |
| **Authenticated account holder** | Valid API key, access to own events | Owner-grade verification; must not obtain other accounts' private metadata via public paths |
| **Insider with log access** | Read application telemetry | Must not be able to reconstruct which hash or proof was queried from public verification logs |

### Attack vectors (public verification perimeter)

- **Hash oracle / enumeration** — submitting many `file_hash` values to learn `exists: true/false` or obtain `public_proof_id`
- **Scraping** — automated harvesting of registration signals from public JSON or PDF responses
- **Timing side-channel** — inferring existence from different code paths or response latency between found and not-found cases
- **Malformed input** — oversized, non-hex, or injection-style parameters to probe errors, bypass validation, or load the database
- **Correlation via telemetry** — recovering evidence identity from audit or access logs if sensitive fields are persisted
- **Proxy header spoofing** — forging client identity for rate limiting when forwarded headers are trusted without a controlled proxy

### Mitigations (current implementation — see §2.4)

- Existence-only disclosure (no ownership, chain topology, or registration cardinality)
- Dedicated public-safe projection (no direct reads of private event storage from public handlers)
- Rate limiting before validation and registry lookup
- Strict format validation with neutral error envelopes
- Unified registry query path for found vs not-found verify responses
- Public verification telemetry without hash, proof id, or raw IP persistence

Mitigations address casual and automated abuse; they do not guarantee protection against determined, distributed adversaries with large IP pools (see §2.2 and §2.4).

---

## 2.2 Security Assumptions

### Transport

- **TLS in transit** protects HTTP traffic between clients and the service in production deployments. Plain HTTP is a deployment misconfiguration, not a supported security mode.

### Cryptography

- **SHA-256** is used for content fingerprints. The system assumes preimage and collision resistance appropriate for evidence fingerprinting.
- **Ed25519** signatures on chain material assume standard elliptic-curve security properties.
- Hashes are **one-way**: public verification confirms registration of a fingerprint, not document content.

### Timestamp authority (TSA)

- **TSA class** (`basic`, `legal`, `vault`, etc.) reflects configured provider trust — not a universal legal guarantee.
- Tier-1 / machine TSA: availability and long-term token retention depend on the external provider.
- Tier-2+ / qualified TSA: stronger operational and jurisdictional assumptions; may be unavailable on some tariff tiers.
- Evident Ledger does **not** operate as a timestamp authority.

### Client and local environment

- Private keys, API keys, local proof files, and optional local document copies are protected by the user's environment.
- Local artifacts (PDFs, ZIP exports, offline `proof.json`) are **verifiable projections** of server-anchored state — see Security Invariant 1.

### Rate limiting scope

- Rate limiting is a **mitigating control** against mass automated probing and casual scraping on a **single instance**.
- It is **not** a hard guarantee against distributed attacks, botnets, or adversaries with large IP address pools.
- Per-instance, in-memory limits do not coordinate across horizontally scaled replicas unless an external shared store is introduced (an implementation detail — see §2.4).

### No content custody

- Original documents are not required to be uploaded. The system operates on cryptographic fingerprints and proof objects.

---

## 2.3 Public API Guarantees

Endpoints (mounted under `/public` in production):

```http
GET /public/verify?file_hash=<sha256-hex>
GET /public/verify/:public_proof_id/certificate.pdf
```

### Disclosure

- **Existence-only** — responses confirm whether a hash is currently registered in the public projection, without revealing ownership, chain topology, or how many independent accounts registered the same hash.
- Public JSON **must not** include: `chain_id`, `event_id`, `merkle_root`, internal signatures, `account_id`, `match_count`, or equivalent registration cardinality.
- Successful verify responses use a **stable schema**; found and not-found differ by field values (e.g. `exists: true/false`), not by HTTP status for the verify query itself.

### Error semantics

- **`429 Too Many Requests`** when rate limits are exceeded — includes `Retry-After` and a neutral `rate_limited` envelope; must not leak counter state or request-specific hash/proof identifiers.
- **`400 Bad Request`** for invalid input format — `invalid_request` envelope with no hint about expected length, alphabet, or field rules (no "hex", "64", "format", etc. in messages).

### Pipeline order (current)

```text
request → rate limit → format validation → public-safe registry lookup → response
```

Invalid and valid requests both consume rate-limit budget once the rate-limit check runs first.

---

## 2.4 Security Controls

**These are current mechanisms.** They may be replaced (e.g. fixed window → token bucket, in-memory → Redis, structured logs → OpenTelemetry) **without** violating Security Invariants (§2.5), provided invariants remain true.

### Rate limiting

- **Algorithm:** fixed window per endpoint (verify vs certificate PDF use independent counters).
- **Scope:** per-instance, in-memory store with TTL eviction and bounded capacity.
- **Key:** `sha256(client_ip)`; optional User-Agent mixing is supported in the interface but disabled by default.
- **Limits (current defaults):** 100 requests/minute/IP on verify; 20 requests/minute/IP on certificate PDF.
- **Proxy headers:** `TRUST_PROXY_HEADERS=false` by default — `X-Forwarded-For` and `X-Real-IP` are ignored unless explicitly enabled for deployments behind a trusted reverse proxy.
- **Ordering:** rate limit runs **before** format validation and **before** registry lookup.

### Format validation

- **`file_hash`:** exactly 64 hexadecimal characters; uppercase input is normalized to lowercase; invalid input returns `400 invalid_request` without database access.
- **`public_proof_id`:** must match the generator format (`pv_` + base58 encoding of 16 bytes) — see `generate_public_id()` in implementation; invalid IDs return `400` before registry lookup.
- Validation runs **after** rate limit, **before** registry lookup.

### Timing mitigation

- **Unified query path:** `exists=true` and `exists=false` for verify use the same registry lookup method and SQL shape — one optional row fetch, no early return that skips the lookup for one branch.
- **Not in scope:** constant-time cryptography, artificial delays (`sleep`), or latency-based guarantees in CI.

### Public verification telemetry

- **Backend:** structured log via `tracing`, target `public_verification_audit` (JSON payload).
- **Fields logged:** timestamp, request type (`verify` | `certificate_pdf`), outcome, rate-limit action, request id, optional `client_ip_hash` (same sha256 scheme as rate limiter).
- **Not persisted in a dedicated audit DB table** at this stage.
- **Forbidden in telemetry:** `file_hash`, `public_proof_id`, raw client IP, boolean `exists`, account identifiers, or any field enabling cross-request correlation to a specific evidence object (see Invariant 8).

---

## 2.5 Security Invariants

These invariants **MUST** remain true across all future releases unless explicitly superseded by a versioned, dated update to this document.
A change to any invariant below is an intentional edit to `SECURITY.md`, not a side effect of a refactor or optimization.

1. The server is the authoritative source of truth for anchored evidence and account ownership. Local artifacts (PDFs, reports, exported proofs) are verifiable projections of server-anchored state, not an independent source of truth.

2. Public verification is existence-only: it confirms that a hash was registered, without disclosing ownership, chain topology, or registration relationships.

3. Public APIs **MUST NOT** disclose `chain_id`, `event_id`, `merkle_root`, internal signatures, account identifiers, or registration cardinality (e.g. how many independent accounts registered the same hash).

4. Public verification **MUST** produce identical disclosure semantics regardless of whether the evidence belongs to one account or to multiple independent accounts.

5. Owner-grade evidence (full chain, Merkle proof, signatures, audit trail) is available only through authenticated account operations or local-first tooling — never through the public web interface.

6. Public verification **MUST** operate through a dedicated public-safe projection and **MUST NOT** query private event storage directly. The specific projection mechanism (table, materialized view, service) is an implementation detail and may change without violating this invariant, as long as the projection itself contains no reversible references to private chain structure.

7. Public verification endpoints **MUST** be protected against unbounded automated probing (mass hash checking, database scraping). The specific mechanism (rate limiting, proof-of-work, CAPTCHA, etc.) is an implementation detail documented in Security Controls (§2.4), not an architectural invariant.

8. Public verification telemetry **MUST NOT** persist file hashes, public proof identifiers, raw client IP addresses, or any other data that could correlate a logged event with a specific evidence object or a specific requester across requests. A change to this invariant requires a dated, explicit update to this document — not an implicit exception in code.

9. `public_proof_id` is an opaque identifier and **MUST NOT** encode, derive from, or otherwise leak internal identifiers, account information, or chain structure.

10. Once anchored, cryptographic evidence is immutable. Corrections, supersessions, or revocations **MUST** be represented as new events and **MUST NOT** modify or delete historical evidence.

11. All security-sensitive changes require a corresponding update to this document (`SECURITY.md`) and, where ownership or API boundaries are affected, to `SYSTEM_CONTRACT.md`, in the same change set.

12. API keys are bearer credentials with the same security requirements as passwords.

13. API keys are never stored in plaintext.

14. Revoked API keys are rejected immediately.

15. Account is the ownership boundary for all resources.

16. Authentication layer **MUST NOT** leak account existence via timing or error messages — except the documented `409 Conflict` on duplicate email at `POST /accounts/register` (see [docs/AUTH_MODEL.md](docs/AUTH_MODEL.md) §3).

17. Subscription status determines API access for paid features.

18. Past-due accounts lose write access immediately.

19. Canceled accounts retain paid-tier access until `current_period_end`; after expiry, `subscription_status` becomes `none` and `tariff_plan_id` reverts to the free tariff.

20. Billing status **MUST NOT** be used to reject free-tier API calls.

21. Webhook signatures **MUST** be verified before any state change.

22. Webhook processing **MUST** be idempotent.

23. Paddle is the source of truth for payment state; local DB is derived state.

24. `tariff_plan_id` always reflects the active plan; `pending_tariff_plan_id` is used for scheduled downgrades.

---

## 2.6 Authentication Model

Full specification: [docs/AUTH_MODEL.md](docs/AUTH_MODEL.md) (frozen at Stage 8.0).

| Surface | Authentication | Notes |
|---------|----------------|-------|
| **Public API** (`/public/*`) | None | Anonymous; rate-limited per §2.4 |
| **Private API** (`/v1/*`) | `X-API-KEY` required | Bearer API key; SHA-256 hash lookup |
| **Account management** (`/accounts/*`) | `X-API-KEY` required | Except `POST /accounts/register` (public bootstrap) |
| **Web sessions** (Stage 8.3) | Cookie-based | Dashboard only; deferred — requires `password_hash` |

**API key storage (normative):** `key_hash = SHA-256(secret)` where `secret` is the 32-hex portion after the `ev_` prefix. Pre-Stage 8.1 legacy keys use `SHA-256(full_key)` — see [docs/AUTH_MODEL.md](docs/AUTH_MODEL.md) §1. Plaintext keys are returned once at creation only.

---

## 2.7 Billing Model

Full specification: [docs/BILLING_MODEL.md](docs/BILLING_MODEL.md) (frozen at Stage 8.2a).

| Concept | Field / rule |
|---------|----------------|
| Active plan | `accounts.tariff_plan_id` — limits, features, enforcement |
| Scheduled downgrade | `accounts.pending_tariff_plan_id` — applies after `current_period_end` |
| Payment state | `accounts.subscription_status` — `none` / `active` / `past_due` / `canceled` |
| Period boundary | `accounts.current_period_end` — paid access window; lazy transitions on first auth request after expiry |
| External authority | Paddle — payment truth; local DB is derived, verified state |
| Free tier | `tariff_plan_id = free` — never blocked by `subscription_status` |

Paid-tier write access requires `active` (or `canceled` before period end). `past_due` blocks writes only on paid tiers. Webhooks: signature verification + idempotency required (Invariants 21–22).

---

## Integrity Guarantees (summary)

Evident Ledger provides:

- Independent verification of evidence records
- Detection of unauthorized modifications
- Cryptographic linkage between events
- Reproducible audit reports

Evident Ledger proves the integrity and existence of recorded data. It does not certify the truthfulness of document contents, the identity of the author without additional identity infrastructure, or legal validity in every jurisdiction.

---

## Responsible Use

Organizations deploying Evident Ledger should maintain appropriate:

- Key management procedures
- Access controls
- Backup strategies
- Compliance processes

Cryptographic evidence is strongest when combined with proper operational security.
