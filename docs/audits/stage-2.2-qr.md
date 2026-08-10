# Stage 2.2 Audit — QR in File Certificate PDF

**Date:** 2026-08-10  
**Scope:** Dependency and embedding strategy only (pre-implementation notes locked to this document).  
**Base:** Stage 2.1 `src/file_certificate_pdf.rs`, tag `v1.3.0-stage2.1`.

---

## 1. Current PDF stack

| Item | Finding |
| --- | --- |
| Library | Root `Cargo.toml` → `printpdf = "0.7"` (line 39). Same stack as `public_certificate_pdf.rs` / Stage 2.1. |
| Document creation | `PdfDocument::new(...)` then builtin Helvetica fonts (`src/file_certificate_pdf.rs` ~190–203). |
| Layout helper | Private `PdfCtx` in `file_certificate_pdf.rs` (lines 45–145): `line`, `bold_line`, `heading`, `field`, `wrap_paragraph`, `finish` → `Result` (no `.expect`). |
| Image API | Stage 2.1 `PdfCtx` has **no** image helpers. `printpdf` supports XObject images, but the established in-repo QR precedent does **not** use images. |
| QR precedent | `vendor/notary-pdf/src/lib.rs` `draw_qr_link` (322–359): `qrcode::QrCode` → dark modules drawn with `layer.add_rect(Rect::new(...))`. Optional URI link annotation (URL-based) — **not** used for Stage 2.2. |

**Decision:** Embed QR as a **vector module grid** (rects), not as a raster image. Keeps the generator pure (no temp files, no image decode path) and matches existing project practice.

---

## 2. QR library choice

| Candidate | Verdict |
| --- | --- |
| **`qrcode` 0.14** | **Selected.** Already used by `vendor/notary-pdf` (`vendor/notary-pdf/Cargo.toml:9`). Pure Rust, no network, no CLI, no system QR tools. |
| External `qrencode` / shell | Rejected — violates “no external binaries / shell”. |
| Online QR APIs | Rejected — network; generator must stay pure. |
| `rqrr` alone | Encoder needed for generation; `rqrr` is a decoder — may be used in **tests** only if decode round-trip is required. |
| Wrapping `notary-pdf::generate_certificate_pdf` | Rejected — different product contract (URL verify link, court-grade template). |

Root `Cargo.toml` currently has **no** direct `qrcode` dependency (only transitively via `notary-pdf`). Stage 2.2 adds `qrcode` to the root package so `file_certificate_pdf` does not depend on notary-pdf internals.

---

## 3. Justification

- **Fits purity:** encode in memory → draw modules on the PDF layer.
- **No second PDF engine;** no image crate required for production path.
- **Aligns with audit Stage 2.0:** QR dep was missing at root; this is the minimal add.
- **Payload policy (product):** QR is **not** a URL. Fixed offline string:

```text
EVIDENT-CERT|{certificate_id}|{registered_merkle_root}
```

Sources: `record.certificate_id` + `proof.proof.root` only. Never `integrity.recomputed_root`, paths, keys, or web endpoints.

---

## 4. Error surface

Production embedding needs one new failure mode: QR encode failure (`QrCode::new`).

```text
CertificateError::QrGeneration(String)
```

No `ImageDecode` — raster path not used.

---

## 5. Out of scope (confirmed)

CLI `verify --event`, GUI, web `/cert`, changes to `write_independent_verification` copy, Stage 1 crypto, `EvidenceRecord` / `ProofFile` schema.
