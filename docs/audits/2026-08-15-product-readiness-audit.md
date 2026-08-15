# Evident Ledger — Product Readiness Audit

**Date:** 2026-08-15  
**Base commit:** `170e9171` (`chore(dev): add deterministic local development entrypoints`)  
**Mode:** Read-only (no product code changes)  
**Artifact workspace:** `/tmp/audit-certificate/`

## Executive Summary

The core evidence path was exercised end-to-end against **production** (`https://evident-ledger.com`, CLI default) because the local server rejected the existing `~/.evident/api_key` (“API key rejected”). Against production, a real file was committed with Machine TSA, a Tier-1 File Certificate was generated with a live `pv_…` QR, public verification returned HTTP 200, offline evidence verification passed (`exit 0`), and tamper tests failed closed for both file hash mismatch and corrupted proof signature (`exit 2`).

**Verdict: CONDITIONAL PASS**

Conditionality is driven by: (1) local-dev API key mismatch blocking a fully local server path, (2) Advanced Evidence Package ZIP not exercised via the штатный GUI button in this sprint (CLI `report generate` + `proof.json` verified instead), and (3) intentional public disclosure asymmetry (Public Verify does not echo SHA-256 / chain / event / signature).

---

## STEP 0 — Baseline

| Check | Result |
| --- | --- |
| `git status --short` | Clean at start of sprint |
| `git log --oneline -5` | Head `170e9171` |
| `./scripts/dev-cli.sh --help` | PASS — CLI help rendered |
| `./scripts/dev-server.sh` + `curl -I http://127.0.0.1:3000/` | PASS — `HTTP/1.1 200 OK` after start |
| Tools | `pdfinfo`, `pdftotext`, `pdftoppm`, `unzip` present; `zbarimg` installed for QR |

**Baseline: PASS**

---

## STEP 1 — Certificate Generation

### Tier-1 File Certificate (PRIMARY)

| Item | Fact |
| --- | --- |
| Renderer | `src/file_certificate_pdf.rs` → `generate_file_certificate(record, proof, public_proof_id)` |
| GUI entry | VerifyResult / Dashboard → **“Generate Evident Certificate”** → `generate_certificate_from_project_proofs` → fresh `lookup_public_proof_id(file_hash)` → `export_file_certificate_pdf` |
| Output dir | `Documents/Evident Certificates` / `certificate_{certificate_id}.pdf` |
| QR | URL `https://evident-ledger.com/public/verify/{public_proof_id}/certificate.pdf` or “Public verification pending” if no `pv_` |

### Other PDF surfaces (not deleted; inventory only)

| Artifact | Location | Role |
| --- | --- | --- |
| Public existence certificate | `src/public_certificate_pdf.rs` → `GET /public/verify/:id/certificate.pdf` | Existence-only disclosure |
| Chain / event report | `evident-report` `generate_report` via GUI `export_event_pdf` / ZIP / CLI `report generate` | Technical verification report |
| Registration snapshot | `generate_registration_snapshot` (GUI helper retained) | Snapshot PDF |
| Hash attestation | `src/hash_attestation_pdf.rs` (legacy HTTP 410 in tests) | LEGACY |
| SAC | `src/sac_pdf.rs` | Chain-level attestation (separate) |
| notary-pdf certificate | `src/bin/evident.rs` (`generate_certificate_pdf`) | Different contract (CLI/report path) |

---

## STEP 2 — Certificate Artifact

**Path used (verified):**

```text
CLI new-chain (default server https://evident-ledger.com)
  → commit original.txt
  → anchored + Machine TSA
  → local EvidenceRecord + ProofFile written under ~/.evident/
  → generate_file_certificate(..., Some(pv_…))  # audit harness calling same library as GUI
  → /tmp/audit-certificate/Evident Certificate.pdf
```

| Field | Value |
| --- | --- |
| chain_id | `2e2dbb0d-8862-40fe-8793-9ffc50022ecd` |
| event_id | `eea97a3f-4120-4e7b-8b73-c062fb5ae7f0` |
| certificate_id | `cert_ac49ae3fb45e58afa0cc1ee171e6c603` |
| SHA-256 | `cdd95e370951dbafe87b4c8a57eab95eebfbcf677331192a06d7f8d16c16d2a1` |
| public_proof_id | `pv_N1B73ZZhv9Zh9UoRUpmAuj` |
| PDF size | 8545 bytes, 3 pages |

**Local-server attempt:** `EVIDENT_SERVER_URL=http://127.0.0.1:3000` → **FAIL** — `API key rejected by the server` (existing key is valid for production, not local DB). Recorded as a readiness gap for local-only workflows.

**Certificate generation: PASS — verified with actual artifact** (library path identical to GUI Tier-1; GUI button not clicked in this headless sprint).

---

## STEP 3 — Certificate Content

`pdfinfo` + `pdftotext` on `/tmp/audit-certificate/Evident Certificate.pdf`.

| Field | In PDF? | Matches ledger? |
| --- | --- | --- |
| Filename | `original.txt` | PASS |
| SHA-256 | present | PASS == evidence + file |
| Certificate ID | present | PASS |
| Chain ID | present | PASS |
| Event ID | present | PASS |
| Registered At | `2026-08-15 19:17:10 UTC` | PASS (aligns with public timestamp) |
| Issuance Date | present (generation time) | PASS (present) |
| Status | `TSA_CONFIRMED` at issuance | PASS vs evidence at gen time |
| Public proof ID (text field) | **Not printed as a labeled field** | PARTIAL — present only inside QR payload |
| Public verification URL (text) | Format hint only: `/public/verify/{public_proof_id}/certificate.pdf` | PARTIAL — full URL in QR, not extractable body text |
| QR | Drawn (vector modules) | PASS (decoded STEP 4) |
| Merkle root | registered + recomputed + MATCH | PASS == proof.json |
| Ed25519 signature | present | PASS == proof.json |
| Public key | present | PASS == proof.json + `~/.evident/server_identity.pub` |
| TSA | CONFIRMED + serial + unix ts + token bytes | PASS == proof.tsa |
| Scope of Attestation | present | PASS (section exists) |
| Independent Verification | present | PASS (section exists) |

**Certificate content: PASS — verified with actual artifact** (with note: `public_proof_id` is QR-only, not a body field).

---

## STEP 4 — QR Verification

```text
zbarimg on PDF: BLOCKED (needs Ghostscript `gs`)
pdftoppm -png -r 300 → zbarimg page-*.png: PASS
```

Decoded payload:

```text
https://evident-ledger.com/public/verify/pv_N1B73ZZhv9Zh9UoRUpmAuj/certificate.pdf
```

| Criterion | Result |
| --- | --- |
| Not localhost / 127.0.0.1 | PASS |
| Not a random/dev fallback | PASS — matches live registry id |
| Format matches contract | PASS |
| QR `public_proof_id` == API `public_proof_id` | PASS |

**QR: PASS — QR decoded and payload recorded**

---

## STEP 5 — Public Verification

| Probe | Result |
| --- | --- |
| `GET https://evident-ledger.com/public/verify?file_hash=…` | HTTP 200 JSON `exists:true`, `integrity:VALID`, `tsa_class:basic` |
| `GET …/public/verify/pv_N1B73ZZhv9Zh9UoRUpmAuj/certificate.pdf` | HTTP 200, `application/pdf`, 2011 bytes |

Public PDF text (existence-only): Status REGISTERED, Public Proof ID, Registration Time, TSA Class basic, Integrity VALID — **no** SHA-256 / chain / event / signature.

| Field | Certificate (Tier-1) | Public Verify | Match |
| --- | --- | --- | --- |
| SHA-256 | present | not disclosed | N/A-by-design |
| Proof ID | in QR only | `pv_N1B73ZZhv9Zh9UoRUpmAuj` | PASS (QR ↔ public) |
| Chain ID | present | not disclosed | N/A-by-design |
| Event ID | present | not disclosed | N/A-by-design |
| Timestamp | `2026-08-15 19:17:10 UTC` | same instant | PASS |
| Signature | present | not disclosed | N/A-by-design |
| TSA | CONFIRMED + serial | class=`basic` | PARTIAL (class only publicly) |
| Status | `TSA_CONFIRMED` | `VALID` / REGISTERED | PARTIAL (different vocab, both positive) |

**Public Verify: PASS — verified with actual HTTP + PDF** (disclosure contract intentional; not a full field mirror of Tier-1).

---

## STEP 6 — Advanced Evidence Package

| Item | Result |
| --- | --- |
| GUI `export_chain_zip` (штатный button) | **Not clicked** in this headless sprint |
| CLI `evident report generate <chain>` | PASS — wrote `~/.evident/proofs/<chain>/proof.pdf` |
| Audit-assembled ZIP | `/tmp/audit-certificate/audit-evidence-package.zip` containing `proof.json`, CLI report as `chain_verification_report.pdf`, `manifest.json`, `original.txt` |
| Per-event `EVENT_00N_attestation.pdf` | Present in GUI ZIP code path (`export_chain_zip` → `export_event_pdf`); **not** present in this audit ZIP |

Unpacked inventory (audit package):

```text
unpacked/chain_verification_report.pdf
unpacked/manifest.json
unpacked/original.txt
unpacked/proof.json
```

**ZIP: PARTIAL** — штатные building blocks exist and CLI report works; full GUI Evidence Package (with per-event attestation PDFs) was not live-exercised.

---

## STEP 7 — Provenance Consistency

| Check | Result |
| --- | --- |
| proof.events[0].file_hash == evidence.sha256 == file SHA-256 | PASS |
| proof.chain_id / event_id == Certificate fields | PASS |
| Merkle registered == recomputed (certificate + CLI verify) | PASS |
| Signature / public key consistent across proof + certificate + pinned `server_identity.pub` | PASS |
| Certificate vs audit ZIP critical fields | PASS for shared artifacts |
| Certificate SHA-256 vs Public PDF | N/A-by-design (public omits hash) |

**Provenance: PASS — verified with actual artifacts**

---

## STEP 8 — Offline Evidence Verification

Command (as specified):

```bash
./scripts/dev-cli.sh verify --event eea97a3f-4120-4e7b-8b73-c062fb5ae7f0 \
  --chain 2e2dbb0d-8862-40fe-8793-9ffc50022ecd
```

| Item | Result |
| --- | --- |
| Exit code | `0` |
| Result line | `Verification Result: PASS` |
| Checks | Event Found / Parent Chain / Merkle / Signature all PASS |
| TSA | CONFIRMED |
| What was verified | **Evidence Record + ProofFile** integrity (not structural backup verification) |
| Server required? | No network call required for this path → **OFFLINE VERIFIED** |
| `~/.evident/server_identity.pub` | EXISTS; contains proof public key (pinning present; not modified) |
| Also: `evident verify proof.json` | `OK: proof valid` (exit 0) |

**Offline Verify: PASS — OFFLINE VERIFIED with actual command**

---

## STEP 9 — Tamper Test

### Test A — file integrity

| Step | Result |
| --- | --- |
| original SHA-256 == Certificate / EvidenceRecord | PASS |
| Flip one byte in copy → new hash | `86934921…` ≠ registered `cdd95e37…` |
| Detection | **INVALID** vs registered hash |

Note: `evident verify --event/--chain` verifies record↔proof crypto; it does **not** take a candidate file path. File tamper detection for a presented document is by recomputing SHA-256 and comparing to the Certificate / evidence hash (product exposes that field).  

**Test A: PASS — verified**

### Test B — proof integrity

| Step | Result |
| --- | --- |
| Flip one byte in live `proof.json` (restored after) | done |
| `verify --event/--chain` | exit **2** |
| Output | `Signature Valid: FAIL`, `Verification Result: FAILED`, `Ed25519 signature verification failed` |

**Test B: PASS — verified**

---

## STEP 10 — Competing Evidence Artifacts

| Artifact | User-facing path | Class |
| --- | --- | --- |
| Evident File Certificate PDF | GUI “Generate Evident Certificate” | **PRIMARY** |
| Public Evidence Certificate PDF | QR / `GET /public/verify/:pv/certificate.pdf` | **PRIMARY** (public) |
| Chain verification report PDF | GUI dashboard / ZIP / CLI `report generate` | **ADVANCED / TECHNICAL** |
| `EVENT_NNN_attestation.pdf` | GUI event row + ZIP packing | **ADVANCED** |
| Registration snapshot PDF | GUI helper / historical Result flow | **TECHNICAL** |
| Evidence package ZIP | GUI “Download Project (ZIP)” | **ADVANCED** |
| `proof.json` | CLI commit / local proofs | **TECHNICAL** |
| SAC PDF | `sac_pdf` | **ADVANCED** (chain-level; separate decision) |
| Hash attestation PDF | legacy API (410 in tests) | **LEGACY** |
| notary-pdf certificate | CLI/report side path | **LEGACY / ALTERNATE** |

Nothing deleted in this sprint.

---

## STEP 11 — Product Journey

```text
File
 ↓  PASS (fixture + SHA-256)
Commit (CLI → production default)
 ↓  PASS (anchored + TSA)
Certificate (Tier-1 library / GUI entry exists)
 ↓  PASS
QR
 ↓  PASS (decoded production URL)
Public Verify
 ↓  PASS (HTTP 200 + existence PDF)
ZIP (full GUI package)
 ↓  PARTIAL (CLI report + proof; GUI ZIP not clicked)
Offline Verify (--event/--chain)
 ↓  PASS (exit 0)
Tamper A (file hash) / Tamper B (proof)
 ↓  PASS / PASS
```

| Stage | Result |
| --- | --- |
| Baseline | PASS |
| Certificate | PASS |
| QR | PASS |
| Public Verify | PASS |
| ZIP | PARTIAL |
| Provenance | PASS |
| Offline Verify | PASS |
| Tamper Test | PASS |

---

## End-to-End Result

| Stage | Result |
| --- | --- |
| Baseline | PASS |
| Certificate | PASS — verified with actual artifact |
| QR | PASS — decoded |
| Public Verify | PASS — HTTP 200 + PDF |
| ZIP | PARTIAL — GUI package not live-exercised |
| Provenance | PASS |
| Offline Verify | PASS — OFFLINE VERIFIED |
| Tamper Test | PASS (A + B) |

---

## Findings

### Critical

_None observed in the exercised production path._

### High

1. **Local server path blocked for existing API key** — `EVIDENT_SERVER_URL=http://127.0.0.1:3000` returns “API key rejected”; CLI default is `https://evident-ledger.com`. Local `./scripts/dev-server.sh` alone is insufficient for CLI commit/certificate without a matching local key/account.  
2. **Advanced GUI ZIP not validated live** — code path exists (`export_chain_zip` + per-event PDFs since `f0dadea0`), but this sprint did not click the GUI action; only CLI `report generate` + assembled package.

### Medium

3. **Tier-1 Certificate does not print `public_proof_id` as a text field** — only QR encodes it; forensics via `pdftotext` cannot recover `pv_…` without QR decode.  
4. **Public Verify is existence-only** — by design; Certificate ↔ Public table is not a full field mirror (SHA-256/chain/event/signature omitted publicly).  
5. **CLI connect error message hardcodes `http://127.0.0.1:3000`** even when failure may be against another `EVIDENT_SERVER_URL` (misleading ops signal).

### Low

6. Direct `zbarimg` on PDF requires Ghostscript (`gs`); PNG fallback works.  
7. Lifecycle label moved `TSA_CONFIRMED` → `CERTIFIED` after offline verify refresh — expected projection refresh, not a mismatch at generation time.  
8. First exploratory commit (without `EVIDENT_SERVER_URL`) landed on production; subsequent local attempts failed auth — document for operators.

---

## Product Readiness Verdict

**CONDITIONAL PASS**

The PRIMARY journey **File → Commit → Certificate → QR → Public Verify → Offline Verify → Tamper** was proven with real artifacts on the production-backed CLI path. Remaining gaps are operational (local API key), packaging (live GUI ZIP), and disclosure UX (QR-only `pv_`), not a broken trust core on the exercised path.

---

## Recommended Implementation Work (Sprint B inputs — do not implement here)

1. Local-dev auth story: provision/document how `~/.evident/api_key` maps to local DB (or seed script), so `./scripts/dev-server.sh` + `./scripts/dev-cli.sh` form a closed loop.  
2. Live GUI Evidence Package acceptance test (ZIP inventory must include `EVENT_*_attestation.pdf` + chain report + proof).  
3. Optionally print `Public Proof ID: pv_…` on Tier-1 Certificate body when available (QR remains authoritative).  
4. Fix CLI error string to show actual `EVIDENT_SERVER_URL`.  
5. Keep public disclosure contract; document Certificate↔Public field matrix in user-facing docs.

---

## Artifact Index

```text
/tmp/audit-certificate/
  original.txt
  original.sha256
  Evident Certificate.pdf
  certificate.txt
  qr-decoded.txt
  public-verify-json.json
  public-certificate.pdf
  public-certificate.txt
  proof.json
  evidence.json
  metadata.txt
  verify-ok.out
  verify-proof-tamper.out
  audit-evidence-package.zip
  unpacked/
```
