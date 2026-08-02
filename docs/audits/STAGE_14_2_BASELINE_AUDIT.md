# Stage 14.2 Baseline Audit

Date: 2026-08-02

Status: **PASS**

Branch: `stage-14.2-baseline-audit`

## Baselines

| Marker | Value |
|--------|-------|
| Release candidate tag | `v1.2.0-rc1` (`303cd9c`) |
| Current HEAD | `fd3dcc7` |

Purpose: classify all changes after `v1.2.0-rc1` through `fd3dcc7` and confirm
cryptographic / API / runtime contracts remain unchanged.

---

## Change classification (`v1.2.0-rc1`..`fd3dcc7`)

Command:

```bash
git diff --stat v1.2.0-rc1..HEAD
git diff --name-only v1.2.0-rc1..HEAD
```

### Documentation-only / sample / contract-lock changes

| Path | Kind |
|------|------|
| `README.md` | Docs (PDF samples + CLI quick start) |
| `ROADMAP.md` | Docs (Vault claim alignment) |
| `SYSTEM_CONTRACT.md` | Docs (Vault status) |
| `docs/API.md` | Docs (TSA boundary clarification) |
| `docs/VERIFY_MODEL.md` | Docs (TSA boundary clarification) |
| `docs/protocol_v0.1.md` | Docs (proof_v1 / TSA model alignment) |
| `docs/audits/PROOF_V1_TSA_ALIGNMENT.md` | Audit (Option A) |
| `docs/audits/STAGE_13_7_IDENTITY_BILLING_FINDING.md` | Audit (deferred debt) |
| `docs/audits/STAGE_13_7_RC_READY.md` | Docs (versioning note) |
| `docs/audits/STAGE_13_7_VAULT_FINDING.md` | Audit |
| `docs/audits/STAGE_14_SIGNING_KEY_READINESS.md` | Audit (Stage 14) |
| `docs/audits/STAGE_14_1_POST_MERGE_VALIDATION.md` | Audit (Stage 14.1) |
| `docs/design/ADR_SIGNING_KEY_GOVERNANCE.md` | ADR (Stage 14) |
| `docs/samples/hash-attestation-66d244d59319785d.pdf` | Sample **removed** (legacy) |
| `docs/samples/public-certificate-sample.pdf` | Sample **added** (public certificate) |
| `tests/proof_schema_contract.rs` | Regression lock for frozen `proof_v1` / TSA boundary |

### Unchanged contract surfaces

| Area | Evidence |
|------|----------|
| Runtime source (`src/`) | No files in `git diff --name-only v1.2.0-rc1..HEAD -- src/` |
| Migrations | None |
| `Cargo.toml` / dependencies | None |
| `docs/proof_v1.schema.md` | Unchanged |
| API behavior (handlers / routes) | No `src/` changes |
| TSA boundary | Documented as external attestation (Option A); no runtime TSA path changes |

---

## Sample PDF note

After `v1.2.0-rc1`, README sample artifacts were corrected:

- Removed legacy `docs/samples/hash-attestation-66d244d59319785d.pdf` (dead
  hash-attestation PDF path, not served by live public routes).
- Added `docs/samples/public-certificate-sample.pdf` generated from live
  `GET /public/verify/:public_proof_id/certificate.pdf`
  (`public_certificate_pdf_handler`).

This is documentation / sample alignment only — not a proof format or API change.

---

## Validation

```bash
cargo test --test proof_schema_contract
```

Result (2026-08-02 @ `fd3dcc7`):

```text
running 3 tests
… ok
test result: ok. 3 passed; 0 failed
```

---

## Result

Post-`v1.2.0-rc1` history through `fd3dcc7` is documentation, audit, sample PDF
replacement, and a proof-schema regression test. Runtime source, `proof_v1`
schema, API handlers, and TSA implementation paths were not modified.
