# Evident Ledger — Roadmap

This document separates **frozen architecture** (requires explicit security review to change) from **evolvable implementation** and **product-layer work**.

Security and verification invariants are defined in [SECURITY.md](SECURITY.md) §2.5. Verification trust layers are defined in [docs/VERIFY_MODEL.md](docs/VERIFY_MODEL.md). System-wide contracts are in [SYSTEM_CONTRACT.md](SYSTEM_CONTRACT.md).

---

## Architecture Freeze

**Frozen (Stage 6.3–7):**

- Verification pipeline (private: auth → ownership → proof → chain → file)
- Public disclosure model (existence-only; no ownership or chain topology)
- Three-layer trust model (proof state / chain integrity / file verification)
- Public API contract:
  - `GET /public/verify?file_hash=…`
  - `GET /public/verify/:public_proof_id/certificate.pdf`
- Security invariants ([SECURITY.md](SECURITY.md) §2.5 — all 11 items)
- Public vs private API boundary ([SYSTEM_CONTRACT.md](SYSTEM_CONTRACT.md) §17)
- Append-only evidence immutability model

Changes that alter any frozen item require a dated update to `SECURITY.md` (and `SYSTEM_CONTRACT.md` / `VERIFY_MODEL.md` where applicable) in the same change set — not as a side effect of refactoring.

---

## Evolvable (implementation detail)

The following may change **without** architecture review, provided [SECURITY.md](SECURITY.md) §2.5 invariants remain satisfied and §2.4 controls are updated if behavior-visible:

- Rate limiting mechanism (fixed window → token bucket; in-memory → Redis; per-instance → distributed)
- Storage engine / schema for the public-safe projection (`public_proof_registry`, materialization tables, views)
- TSA providers and tariff-to-`tsa_class` mapping
- Telemetry / audit backend (structured `tracing` logs → OpenTelemetry, SIEM, dedicated audit store) — subject to Invariant 8
- Proxy header trust policy defaults (with deployment documentation)
- Exact rate-limit numeric thresholds

---

## Product Layer (next)

Work after the architecture freeze. Does not relax security invariants without explicit documentation updates.

- **Vault / Backup** — encrypted server backup, restore workflows
- **Identity / Signer** — user identity binding, enhanced trust tiers
- **Billing / Tariffs** — plan enforcement, qualified TSA availability
- **External integrations** — third-party APIs, webhooks
- **GUI** — unified storage model, product UX (see [SYSTEM_CONTRACT.md](SYSTEM_CONTRACT.md) §2.3 for current GUI/CLI separation)
