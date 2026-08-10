# Stage 2.5 Audit — GUI File Certificate PDF Export

**Date:** 2026-08-10  
**Base:** Stage 2.4.1 (`v1.3.0-stage2.4.1`)  
**Scope:** GUI-only wiring of existing File Certificate renderer. No CLI / API / DB / lifecycle / verify / QR / renderer changes.

---

## 1. GUI action

| Item | Location |
| --- | --- |
| Action label | `Generate Certificate PDF` |
| Verify result screen | `evident-gui-app/src/main.rs` — beside Download Report (PDF) on `Screen::VerifyResult` |
| Commit result screen | Same file — beside Verify / Open PDF on `Screen::Result` |
| Navigation | Unchanged (no new screen) |

Status is tracked separately from integrity verification via:

```rust
enum CertificateStatus {
    None,
    Generating,
    Generated(PathBuf),
    Failed(String),
}
```

`VerifyStatus` remains the integrity-check outcome only.

---

## 2. Data sources (existing APIs only)

| Input | API | Path / notes |
| --- | --- | --- |
| `EvidenceRecord` | `evident_ledger::evidence_record::read_evidence_record(dir, evidence_id)` | `default_evidence_dir()` → `~/.evident/evidence/` (via `evidence_id_for_event`) |
| `ProofFile` | `App::load_local_proof(&proofs_dir)` | Project `…/proofs/` (unchanged selection: largest event-count snapshot) |
| PDF bytes | `evident_ledger::file_certificate_pdf::generate_file_certificate(&record, &proof)` | Stage 2.1/2.2 renderer — **not modified** |

Mismatch between selected proof and current evidence (`chain_id` / event / hash) fails with a clear `CertificateStatus::Failed` message (no panic).

GUI does **not** parse evidence JSON with `serde_json::from_str` for this path.

---

## 3. Renderer / QR unchanged

| Constraint | Status |
| --- | --- |
| `src/file_certificate_pdf.rs` | Not modified |
| QR payload | Still Stage 2.2: `EVIDENT-CERT\|{certificate_id}\|{registered_merkle_root}` (no URL) |
| `CertificateError` | Unchanged |
| `EvidenceRecord` / `ProofFile` formats | Unchanged |
| `read_evidence_record` / `load_local_proof` signatures | Unchanged |

---

## 4. Save path

```text
dirs::document_dir() / "Evident Certificates"
```

Fallback when `document_dir()` is unavailable:

```text
$HOME/Documents/Evident Certificates
```

via `std::env::var("HOME")`. If both fail, user sees a clear error.

Filename:

```text
certificate_{certificate_id}.pdf
```

Collision policy: never overwrite; use `certificate_{id}_1.pdf`, `_2.pdf`, … via `create_dir_all` + existence checks.

---

## 5. Network / API

Export path is local-only: read evidence from disk, load local proof, generate PDF in-process, write under Documents. No network call and no API/database dependency for Stage 2.5 certificate export.

---

## 6. Tests (GUI crate)

In `evident-gui-app/src/main.rs` (`stage25_certificate_export_tests`):

1. PDF export success → nonempty file on disk  
2. Exported PDF contains Stage 2.2 QR block label / format hint  
3. Existing `certificate_test.pdf` → next name `certificate_test_1.pdf` (original untouched)  
4. Missing EvidenceRecord / ProofFile → `Failed(...)` without panic  

---

## 7. Allowed vs forbidden diff (DoD)

**Allowed:** `evident-gui-app/src/main.rs`, this audit, GUI-local tests.  

**Forbidden (verified by scope):** `src/file_certificate_pdf.rs`, `src/evidence_record.rs`, root `Cargo.toml` / lock, CLI, API, database, lifecycle, verify algorithm.
