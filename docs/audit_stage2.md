# Stage 2 Audit — File Certificate PDF (pre-implementation)

**Date:** 2026-08-10  
**Scope:** Task 2.0 only — factual inventory before any Stage 2 code.  
**Base:** Stage 1 Evidence Record layer (`src/evidence_record.rs`, `docs/audit_stage1.md`).  
**Constraint for this document:** no Rust / Cargo / GUI / CLI changes.

---

## 1. PDF dependency (already in tree)

| Location | Dependency | Role |
| --- | --- | --- |
| Root `Cargo.toml:39` | `printpdf = "0.7"` | Direct PDF renderer used by in-crate modules |
| Root `Cargo.toml:38` | `notary-pdf = { path = "vendor/notary-pdf" }` | Higher-level certificate PDF (court-grade template) |
| `vendor/notary-pdf/Cargo.toml:8-9` | `printpdf = "0.7"`, `qrcode = "0.14"` | Vendored PDF + QR |
| `evident-report/Cargo.toml` | `printpdf = "0.7"` | GUI/CLI evidence reports |
| `evident-gui-app/Cargo.toml:13` | `notary-pdf` path dep | GUI can call notary-pdf |

**Verdict for Stage 2.1:** use the same stack as `src/public_certificate_pdf.rs` — **`printpdf` already on the root crate**. Do **not** add a second PDF engine.

`notary-pdf::generate_certificate_pdf` is a **different product contract** (Russian court-grade layout, `CertificateStatus::Valid` / hardcoded semantics, requires `CertificateInput` with `tsa_provider`, `verify_url`, etc.). It is **not** a drop-in for File Certificate over `EvidenceRecord` + `ProofFile`. Prefer a sibling module patterned on `public_certificate_pdf.rs`, not wrapping `notary-pdf`.

---

## 2. What `src/public_certificate_pdf.rs` actually is

**Important correction vs Stage 2 TZ wording:** this file is **not** a chain-level ledger certificate. It renders a **public existence** certificate from `PublicRegistryEntry` (Stage 6.4), used by `GET /public/verify/:public_proof_id/certificate.pdf` (`src/api/public_verify.rs` → `render_public_certificate_pdf`).

Chain-level / richer PDFs live elsewhere:

| Module | Purpose |
| --- | --- |
| `src/public_certificate_pdf.rs` | Public hash-registry existence PDF |
| `src/sac_pdf.rs` | SAC (chain state attestation) PDF |
| `src/hash_attestation_pdf.rs` | Multi-chain hash attestation PDF |
| `evident-report` (`generate_report`, …) | GUI/CLI evidence snapshot & event attestation PDFs |

### Line-by-line structure of `public_certificate_pdf.rs`

| Lines | Content |
| --- | --- |
| 1–3 | Module docs: public cert from `public_proof_id` only |
| 5–6 | `use crate::public_proof::PublicRegistryEntry`; `use printpdf::*` |
| 9–14 | Page constants: A4 mm (`210×297`), margins, `LINE_HEIGHT = 6.0` |
| 16–22 | Private `PdfCtx { doc, layer, font, bold, y }` |
| 24–38 | `PdfCtx::new` — starts `y` at top margin |
| 40–49 | `line(&mut self, text, size)` — auto page-break, Helvetica |
| 51–60 | `bold_line` — same with HelveticaBold |
| 62–71 | `finish(self) -> Vec<u8>` — `doc.save` into buffer; **panics on save failure** via `.expect(...)` |
| 74–106 | `pub fn render_public_certificate_pdf(entry: &PublicRegistryEntry) -> Vec<u8>` |
| 75–86 | `PdfDocument::new`, builtin fonts, `PdfCtx` |
| 88–103 | Content: title, hardcoded `"Status: REGISTERED"`, public id, registration time, TSA class, integrity, disclaimer |
| 105 | `ctx.finish()` |
| 108–127 | Unit test: bytes start with `%PDF` |

### Can Stage 2 reuse layout helpers?

**Not by import.** `PdfCtx`, `line`, `bold_line` are **private** to this module (no `pub`). Options for 2.1:

1. **Copy the same private `PdfCtx` pattern** into `src/file_certificate_pdf.rs` (lowest coupling; matches “don’t invent a new PDF stack”).
2. Later extract a shared `pdf_layout` helper (out of Stage 2 minimal scope unless needed).

There is **no** image/QR helper in `public_certificate_pdf.rs`. QR drawing precedent is private inside `vendor/notary-pdf` (`draw_qr_link`, lines 322–359): modules → `layer.add_rect` + optional link annotation.

---

## 3. Proposed module location

**Yes:** `src/file_certificate_pdf.rs`, registered next to peers in `src/lib.rs` (today: `pub mod public_certificate_pdf` at line 17; `pub mod evidence_record` at line 7).

Suggested public API (Stage 2.1 — not implemented yet):

```rust
pub fn generate_file_certificate(
    record: &EvidenceRecord,
    proof: &ProofFile,
) -> Result<Vec<u8>, CertificateError>
```

`CertificateError` does **not** exist yet in the crate (grep: no matches). Stage 2 will introduce it in this module (or a small enum colocated with the generator).

---

## 4. Existing GUI PDF flows (verified in `evident-gui-app/src/main.rs`)

### 4a. Per-event row button `📄 PDF` (Dashboard verify list)

| Step | Lines | Behavior |
| --- | --- | --- |
| Button | 3442–3447 | `egui::Button::new("📄 PDF")` in event row |
| Click | 3449–3495 | Requires `self.last_proof`; else error “run verification first” |
| Generator | 3452–3457 → `Self::export_event_pdf(...)` | |
| Implementation | 789–897 | Builds `ProofData` + `VerificationContext` from **in-memory** `ProofFile` + `VerificationEvent` |
| Library | 894–895 | `evident_report::generate_report(...)` → `{project}/proofs/EVENT_{seq}_attestation.pdf` |
| Open | 3459–3464 | `Command::new("open").arg(&pdf_path)` |

**This is already an event-level PDF**, but it is **not** the Stage 2 File Certificate:

- Does **not** read `EvidenceRecord` from `~/.evident/evidence/`.
- Does **not** call `verify_evidence_integrity`.
- Does **not** display `certificate_id` / `lifecycle_status` from Stage 1.
- Uses `evident-report` attestation layout, not `printpdf`/`file_certificate_pdf`.

**Stage 2.5 implication:** rewiring this button to `generate_file_certificate` **replaces** the current `export_event_pdf` behavior for that control. That is an intentional product change, not a greenfield button — document regression risk (see § Risks).

### 4b. Chain-level `📄 Download Report (PDF)` — do not touch (per TZ 2.5)

| Step | Lines | Behavior |
| --- | --- | --- |
| Button | 3547–3551 | “Download Report (PDF)” |
| Proof refresh | 3554–3561 | Optional `fetch_proof` then fallback to `last_proof` |
| Build | 3575–3579 | `build_evidence_snapshot` |
| Generate | 3582–3586 | `generate_report` → `evidence_snapshot.pdf` |

### 4c. `📋 Full Audit Chain` — navigation, not PDF

| Lines | Behavior |
| --- | --- |
| 4086–4096 | Switches to `Screen::VerifyProject` and runs `verify_project` — **not** a PDF generator |

### 4d. ZIP on event row — already disabled placeholder

| Lines | Behavior |
| --- | --- |
| 3429–3440 | Disabled “ZIP (soon)” — aligns with Stage 2 out-of-scope for ZIP |

### Recommended Stage 2.5 pattern

Copy the **control flow** of 3442–3495 (click → load data → generate bytes/path → `open` → status string), but:

1. Resolve `event_id` / `chain_id` from the row.
2. **Re-read** `EvidenceRecord` from disk (`read_evidence_record` / path via `evidence_id_for_event`).
3. Load `ProofFile` from `~/.evident/proofs/{chain_id}/{event_id}.json` (and/or project proofs — see risk on dual proof locations).
4. Call `generate_file_certificate(record, proof)`.
5. Leave 3547+ chain report and `export_event_pdf` unused by this button (or keep `export_event_pdf` only if product wants both — TZ says replace per-event PDF with File Certificate).

---

## 5. QR dependency

| Question | Answer |
| --- | --- |
| Is `qrcode` in **root** `Cargo.toml`? | **No** (deps end at `async-trait`; see lines 16–44). |
| Exists elsewhere? | Yes: `vendor/notary-pdf/Cargo.toml:9` → `qrcode = "0.14"`, used privately in `draw_qr_link` (`vendor/notary-pdf/src/lib.rs:322-359`). |
| Public QR API on `notary-pdf`? | **No.** Only `generate_certificate_pdf` is public; QR is internal. |

**Stage 2.2 will require a Cargo.toml change** (add `qrcode` to the root crate, or a small shared helper) — **not done in 2.0**.

### QR URL domain (audit)

| Candidate | Status in repo |
| --- | --- |
| `https://verify.evident-ledger.com/cert/{certificate_id}` | TZ placeholder; **no** such host/route in code |
| Production API/site | `https://evident-ledger.com` (CLI/GUI default `EVIDENT_SERVER_URL`) |
| Existing public verify | `GET /public/verify?file_hash=…` and `…/:public_proof_id/certificate.pdf` — keyed by **hash / `pv_…`**, **not** `certificate_id` |

**Recommendation for 2.2:** encode the TZ URL (or `https://evident-ledger.com/cert/{certificate_id}`) with the mandatory “coming soon / use CLI” disclaimer. Do **not** imply `/cert/{id}` is live.

---

## 6. Real Stage 1 types & functions (signatures)

### `EvidenceRecord` — `src/evidence_record.rs:51-69`

Fields: `evidence_id`, `filename: Option<String>`, `sha256`, `size_bytes: Option<u64>`, `mime_type: Option<String>`, `local_file_available: bool`, `chain_id`, `event_id`, `certificate_id`, `created_at_local`, `registered_at`, `lifecycle_status`, `tsa_status`, `project_id: Option<String>`, `proof_path: Option<String>`.

Enums (`SCREAMING_SNAKE_CASE` serde):

- `LifecycleStatus` (31–38): `Created`, `Registered`, `TsaConfirmed`, `Certified`, `Revoked`
- `TsaStatus` (42–47): `Pending`, `Confirmed`, `Failed`, `Absent`

No `Display` impl — PDF should format via `serde` name or explicit match (`TSA_CONFIRMED`, etc.).

### `ProofFile` — `src/client.rs:48-56`

```text
leaf_version, chain_id, head_event_id,
proof: ProofPayload { root, chain_head, signature, public_key, leaves_count, version?, type? },
events: Vec<EventLeaf>,
tsa: Option<TsaData>
```

### `TsaData` — `src/client.rs:8-12`

```text
timestamp: Option<i64>, serial: Option<String>, token_bytes: Option<i64>
```

**No `provider` field** on `ProofFile.tsa`. Server-side `TsaAttestation.provider` (`src/tsa/types.rs:5`) is a **different** type, not present in local proof JSON.

### `EvidenceIntegrityResult` — `src/evidence_record.rs:81-88`

`event_found`, `parent_chain_valid`, `merkle_root_valid`, `signature_valid`, `recomputed_root: Option<String>`, `errors: Vec<String>`; `is_valid()` at 91–97.

### Functions

| Function | Lines | Signature |
| --- | --- | --- |
| `evidence_id_for_event` | 101–103 | `fn(Uuid) -> String` → `ev_{uuid.as_simple()}` (no hyphens) |
| `certificate_id_for_event` | 106–109 | `fn(Uuid) -> String` → `cert_{uuidv5.as_simple()}` |
| `write_evidence_record` | 173–179 | `fn(dir: &Path, record: &EvidenceRecord) -> io::Result<PathBuf>` |
| `read_evidence_record` | 182–187 | `fn(dir: &Path, evidence_id: &str) -> io::Result<EvidenceRecord>` |
| `verify_evidence_integrity` | 248–251 | `fn(record: &EvidenceRecord, proof: &ProofFile) -> EvidenceIntegrityResult` |
| `derive_lifecycle` | 221–224 | `fn(integrity: &EvidenceIntegrityResult, tsa_status: TsaStatus) -> LifecycleStatus` |
| `refresh_lifecycle` | 238–242 | `fn(record: &mut EvidenceRecord, proof: &ProofFile)` — mutates status in memory only |

**Note:** `refresh_lifecycle` does **not** write disk; Stage 2.4 must call `write_evidence_record` after it.

### Confirmed E2E fixture on this machine

| Item | Value |
| --- | --- |
| Evidence file | `~/.evident/evidence/ev_22d29a6a4cb4469fbbce1d07e49694ce.json` |
| `lifecycle_status` | `TSA_CONFIRMED` |
| `tsa_status` | `CONFIRMED` |
| `certificate_id` | `cert_8863fcee635a583ebce87ca80af729db` |
| `event_id` | `22d29a6a-4cb4-469f-bbce-1d07e49694ce` |
| `chain_id` | `c0bafd33-6807-4fb7-b480-c454ecabdd5d` |
| Proof file | `~/.evident/proofs/c0bafd33-6807-4fb7-b480-c454ecabdd5d/22d29a6a-4cb4-469f-bbce-1d07e49694ce.json` |

---

## 7. CLI today — how to add `verify --event` without breaking existing commands

### Current `verify` dispatch — `src/bin/evident.rs:397-423`

```text
evident verify <proof.json>
evident verify --chain <chain_id>   → find_latest_proof_artifact → cmd_verify(path)
```

`cmd_verify` (761–783) does **not** call `verify_evidence_integrity`. It shells out to the **`evident-verify` binary** on a proof JSON path.

### Safe extension pattern for Stage 2.3

Extend the `Some("verify")` arm to parse flags **before** treating the first token as a path:

```text
evident verify --event <uuid> --chain <uuid>   → new cmd (EvidenceRecord + ProofFile + verify_evidence_integrity)
evident verify --chain <uuid>                  → existing latest-proof → evident-verify
evident verify <proof.json>                    → existing
```

Parsing rules that avoid breakage:

1. If first arg is `--event` (or both `--event` / `--chain` appear in the remaining argv), take the new path.
2. Else if first arg is `--chain`, keep today’s behavior.
3. Else treat as proof path (today).
4. Update `print_verify_help` / global help (269–281, 232–233).

Path resolution for the new mode:

- Evidence: `~/.evident/evidence/{evidence_id_for_event(event_uuid)}.json`  
  (`ev_` + **simple** UUID — hyphens stripped; do not naively concatenate dashed UUID).
- Proof: `~/.evident/proofs/{chain_id}/{event_id}.json` (dashed UUID as filename — matches on-disk layout).

Missing files → `CliError` with the expected absolute path (TZ 2.3).

Stage 2.4: after integrity check, `refresh_lifecycle` + `write_evidence_record(default_evidence_dir(), &record)`.

---

## 8. Event-level PDF / endpoints already present?

| Surface | Exists? | Notes |
| --- | --- | --- |
| GUI per-event PDF | **Yes** | `export_event_pdf` + `evident-report` |
| `file_certificate_pdf.rs` | **No** | Proposed only |
| HTTP File Certificate by `certificate_id` | **No** | Public PDF is by `public_proof_id` |
| CLI `verify --event` | **No** | Only proof path / `--chain` |

---

## 9. What Stage 2 will need in Cargo.toml (deferred — not changed in 2.0)

| Need | Already present? | Action in later tasks |
| --- | --- | --- |
| PDF via `printpdf` | Yes (`Cargo.toml:39`) | No new PDF crate |
| QR via `qrcode` | Only under `vendor/notary-pdf` | **Add** `qrcode` to root crate for 2.2 **or** extract/reuse helper from notary-pdf (would touch vendor API) |
| `uuid` v5 | Yes (Stage 1) | No change |
| New crates for integrity / lifecycle / CLI | No | None required |

### Implementable **without** new dependencies

- 2.1 PDF body (text sections) with `printpdf` + copy of `PdfCtx` pattern  
- 2.3 CLI `verify --event --chain`  
- 2.4 `refresh_lifecycle` + `write_evidence_record`  
- 2.5 GUI wiring (call generator; save/open file)  
- 2.6 unit tests for PDF bytes / lifecycle / missing paths  

### Requires new dependency (or vendor API change)

- 2.2 QR rasterization (`qrcode`), unless Stage 2 deliberately routes through an expanded `notary-pdf` public helper (still a code change beyond root-only `printpdf`).

---

## 10. Answers checklist (Task 2.0 acceptance)

1. **PDF library:** `printpdf` 0.7 (root); precedent module `src/public_certificate_pdf.rs`. Also `notary-pdf` / `evident-report` for other products — do not mix contracts.
2. **Layout helpers:** private `PdfCtx::line` / `bold_line` / `finish` at lines 40–71 — **copy pattern**, cannot import.
3. **New module path:** `src/file_certificate_pdf.rs` (+ `pub mod` in `lib.rs`).
4. **GUI PDF pattern:** event-row `📄 PDF` → `export_event_pdf` (3442–3495 / 789–897); chain report separate (3547–3586). Stage 2.5 should mirror click→generate→open, but target File Certificate + disk `EvidenceRecord`.
5. **QR:** not in root `Cargo.toml`; present only via `vendor/notary-pdf` privately → Stage 2.2 needs an explicit dependency decision.

---

## Architectural risks / contradictions for Stage 2 implementation

1. **TZ called `public_certificate_pdf` “chain-level”** — code shows **public existence** cert. Real chain/event attestation PDFs are `sac_pdf` / `evident-report`. File Certificate should still follow `public_certificate_pdf`’s **printpdf/`PdfCtx` style**, not its data model (`PublicRegistryEntry`).

2. **Per-event GUI `📄 PDF` already generates a different PDF** (`EVENT_*_attestation.pdf` via `evident-report`). Stage 2.5 rewires that button → product change / possible user surprise; keep chain “Download Report” untouched as specified.

3. **`TsaData` has no `provider`** — Stage 2.1 text asking for `provider/serial/timestamp` from `proof.tsa` can only honestly print `serial` + `timestamp` (and maybe token length). Printing a fake provider violates Stage 1 honesty rules. Use `"(not in proof artifact)"` or omit provider.

4. **Dual proof locations:** CLI/Evidence Record point at `~/.evident/proofs/...`; GUI also keeps project-local `…/projects/{name}/proofs/{event_id}.json`. Stage 2.5 must define which path is authoritative for File Certificate (prefer `record.proof_path` if set, else `~/.evident/proofs/...`).

5. **`refresh_lifecycle` alone does not persist** — forgetting `write_evidence_record` in 2.4 leaves PDF status stuck at `TSA_CONFIRMED`.

6. **`CERTIFIED` requires `TsaStatus::Confirmed` + full integrity** (`derive_lifecycle` 231–234). CLI output “Overall: CERTIFIED” must come from **updated** `lifecycle_status` after refresh, not invent a parallel status.

7. **Existing `evident verify` ≠ `verify_evidence_integrity`** — different code path (`evident-verify` binary). New `--event` mode must not silently change the old proof-path behavior.

8. **QR URL `/cert/{certificate_id}` is not implemented** — must ship with “coming soon” disclaimer (TZ 2.2); otherwise a scannable dead link.

9. **`PdfCtx::finish` uses `.expect`** — Stage 2 should prefer `Result`/`CertificateError` for the new generator (TZ signature is `Result<…>`), not copy the panic-on-save style.

10. **Evidence id formatting:** `evidence_id_for_event` strips UUID hyphens. CLI/`--event` parsers must use that helper, not `format!("ev_{event_id}")` with dashed UUIDs.

---

## Explicitly out of scope (confirm)

- No public `/cert/{id}` verifier (Stage 4).
- No `evidence_package.zip` / ZIP button enablement.
- No Merkle inclusion path / siblings.
- No changes in this audit task to Cargo.toml, Rust, GUI, or CLI.
