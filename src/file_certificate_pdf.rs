//! File Certificate PDF (Stage 2.1).
//!
//! Pure renderer over [`EvidenceRecord`] + [`ProofFile`]. Does not touch the
//! network, database, filesystem, or GUI. Verification uses Stage 1
//! [`verify_evidence_integrity`] only — no parallel crypto.

use crate::client::ProofFile;
use crate::evidence_record::{
    verify_evidence_integrity, EvidenceIntegrityResult, EvidenceRecord, LifecycleStatus, TsaStatus,
};
use chrono::Utc;
use printpdf::*;
use std::fmt;
use std::io::Cursor;

const PAGE_WIDTH: f32 = 210.0;
const PAGE_HEIGHT: f32 = 297.0;
const MARGIN_LEFT: f32 = 20.0;
const MARGIN_TOP: f32 = 25.0;
const MARGIN_BOTTOM: f32 = 20.0;
const LINE_HEIGHT: f32 = 6.0;
const SECTION_GAP: f32 = 8.0;
const WRAP_CHARS: usize = 78;

const VERIFICATION_MODEL: &str = "Full leaf-set Merkle root recomputation (parent-chain + merkle-root-v1). No per-event inclusion path — see Known Limitation in docs/audit_stage1.md.";

/// Errors from File Certificate PDF generation (layout / PDF I/O only).
#[derive(Debug)]
pub enum CertificateError {
    Font(String),
    PdfSave(String),
}

impl fmt::Display for CertificateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Font(msg) => write!(f, "PDF font error: {msg}"),
            Self::PdfSave(msg) => write!(f, "PDF save error: {msg}"),
        }
    }
}

impl std::error::Error for CertificateError {}

struct PdfCtx {
    doc: PdfDocumentReference,
    layer: PdfLayerReference,
    font: IndirectFontRef,
    bold: IndirectFontRef,
    y: f32,
}

impl PdfCtx {
    fn new(
        doc: PdfDocumentReference,
        layer: PdfLayerReference,
        font: IndirectFontRef,
        bold: IndirectFontRef,
    ) -> Self {
        Self {
            doc,
            layer,
            font,
            bold,
            y: PAGE_HEIGHT - MARGIN_TOP,
        }
    }

    fn ensure_space(&mut self, lines_needed: f32) {
        let needed = LINE_HEIGHT * lines_needed;
        if self.y - needed < MARGIN_BOTTOM {
            let (page, layer) = self.doc.add_page(Mm(PAGE_WIDTH), Mm(PAGE_HEIGHT), "Layer");
            self.layer = self.doc.get_page(page).get_layer(layer);
            self.y = PAGE_HEIGHT - MARGIN_TOP;
        }
    }

    fn line(&mut self, text: &str, size: f32) {
        self.ensure_space(1.0);
        self.layer
            .use_text(text, size, Mm(MARGIN_LEFT), Mm(self.y), &self.font);
        self.y -= LINE_HEIGHT;
    }

    fn bold_line(&mut self, text: &str, size: f32) {
        self.ensure_space(1.0);
        self.layer
            .use_text(text, size, Mm(MARGIN_LEFT), Mm(self.y), &self.bold);
        self.y -= LINE_HEIGHT;
    }

    fn heading(&mut self, text: &str) {
        self.ensure_space(2.4);
        self.y -= SECTION_GAP;
        self.layer
            .use_text(text, 12.0, Mm(MARGIN_LEFT), Mm(self.y), &self.bold);
        self.y -= LINE_HEIGHT * 1.4;
    }

    fn gap(&mut self) {
        self.y -= SECTION_GAP * 0.5;
    }

    fn field(&mut self, label: &str, value: &str) {
        if label.len() + 1 + value.len() <= WRAP_CHARS {
            self.line(&format!("{label} {value}"), 10.0);
            return;
        }
        self.line(label, 10.0);
        let chars: Vec<char> = value.chars().collect();
        for chunk in chars.chunks(WRAP_CHARS.saturating_sub(4).max(8)) {
            let piece: String = chunk.iter().collect();
            self.line(&format!("    {piece}"), 10.0);
        }
    }

    fn wrap_paragraph(&mut self, text: &str) {
        let mut current = String::new();
        for word in text.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.len() + 1 + word.len() <= WRAP_CHARS {
                current.push(' ');
                current.push_str(word);
            } else {
                self.line(&current, 9.5);
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            self.line(&current, 9.5);
        }
    }

    fn finish(self) -> Result<Vec<u8>, CertificateError> {
        let mut buffer: Vec<u8> = Vec::new();
        {
            let mut writer = std::io::BufWriter::new(Cursor::new(&mut buffer));
            self.doc
                .save(&mut writer)
                .map_err(|e| CertificateError::PdfSave(e.to_string()))?;
        }
        Ok(buffer)
    }
}

fn lifecycle_label(status: LifecycleStatus) -> &'static str {
    match status {
        LifecycleStatus::Created => "CREATED",
        LifecycleStatus::Registered => "REGISTERED",
        LifecycleStatus::TsaConfirmed => "TSA_CONFIRMED",
        LifecycleStatus::Certified => "CERTIFIED",
        LifecycleStatus::Revoked => "REVOKED",
    }
}

fn tsa_status_label(status: TsaStatus) -> &'static str {
    match status {
        TsaStatus::Pending => "PENDING",
        TsaStatus::Confirmed => "CONFIRMED",
        TsaStatus::Failed => "FAILED",
        TsaStatus::Absent => "ABSENT",
    }
}

fn bool_label(v: bool) -> &'static str {
    if v {
        "true"
    } else {
        "false"
    }
}

fn merkle_comparison(registered: &str, recomputed: Option<&str>) -> &'static str {
    match recomputed {
        Some(root) if root == registered => "MATCH",
        Some(_) => "MISMATCH",
        None => "NOT AVAILABLE",
    }
}

/// Generate a File Certificate PDF from an Evidence Record projection and its
/// linked proof artifact. Pure: no I/O beyond PDF serialization.
pub fn generate_file_certificate(
    record: &EvidenceRecord,
    proof: &ProofFile,
) -> Result<Vec<u8>, CertificateError> {
    let integrity = verify_evidence_integrity(record, proof);

    let (pdf_doc, page1, layer1) = PdfDocument::new(
        "Evident Ledger File Certificate",
        Mm(PAGE_WIDTH),
        Mm(PAGE_HEIGHT),
        "Layer 1",
    );
    let font = pdf_doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .map_err(|e| CertificateError::Font(e.to_string()))?;
    let bold = pdf_doc
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .map_err(|e| CertificateError::Font(e.to_string()))?;
    let layer = pdf_doc.get_page(page1).get_layer(layer1);
    let mut ctx = PdfCtx::new(pdf_doc, layer, font, bold);

    write_header(&mut ctx, record);
    write_target_file(&mut ctx, record);
    write_ledger_registration(&mut ctx, record);
    write_integrity_checks(&mut ctx, &integrity);
    write_verification_errors(&mut ctx, &integrity);
    write_crypto_and_time(&mut ctx, record, proof, &integrity);
    write_verification_model(&mut ctx);
    write_scope_of_attestation(&mut ctx);
    write_independent_verification(&mut ctx);

    ctx.finish()
}

fn write_header(ctx: &mut PdfCtx, record: &EvidenceRecord) {
    ctx.bold_line("EVIDENT LEDGER — FILE CERTIFICATE", 14.0);
    ctx.gap();
    ctx.field("Certificate ID:", &record.certificate_id);
    ctx.field("Status:", lifecycle_label(record.lifecycle_status));
    ctx.field(
        "Issuance Date:",
        &Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
    );
}

fn write_target_file(ctx: &mut PdfCtx, record: &EvidenceRecord) {
    ctx.heading("Target File Information");
    ctx.field(
        "Filename:",
        record.filename.as_deref().unwrap_or("(not provided)"),
    );
    match record.size_bytes {
        Some(n) => ctx.field("Size:", &format!("{n} bytes")),
        None => ctx.field("Size:", "(unknown)"),
    }
    ctx.field(
        "MIME Type:",
        record.mime_type.as_deref().unwrap_or("(unknown)"),
    );
    ctx.field("SHA-256:", &record.sha256);
    let local = if record.local_file_available {
        "AVAILABLE"
    } else {
        "NOT AVAILABLE ON THIS DEVICE"
    };
    ctx.field("Local file status:", local);
}

fn write_ledger_registration(ctx: &mut PdfCtx, record: &EvidenceRecord) {
    ctx.heading("Ledger Registration Details");
    ctx.field("Chain ID:", &record.chain_id);
    ctx.field("Event ID:", &record.event_id);
    ctx.field(
        "Registered At:",
        &record
            .registered_at
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string(),
    );
}

fn write_integrity_checks(ctx: &mut PdfCtx, integrity: &EvidenceIntegrityResult) {
    ctx.heading("Integrity Verification");
    ctx.field("Event Found:", bool_label(integrity.event_found));
    ctx.field(
        "Parent Chain Valid:",
        bool_label(integrity.parent_chain_valid),
    );
    ctx.field("Merkle Root Valid:", bool_label(integrity.merkle_root_valid));
    ctx.field("Signature Valid:", bool_label(integrity.signature_valid));
}

fn write_verification_errors(ctx: &mut PdfCtx, integrity: &EvidenceIntegrityResult) {
    ctx.heading("Verification Errors");
    if integrity.errors.is_empty() {
        ctx.line("(none)", 10.0);
    } else {
        for err in &integrity.errors {
            ctx.wrap_paragraph(err);
        }
    }
}

fn write_crypto_and_time(
    ctx: &mut PdfCtx,
    record: &EvidenceRecord,
    proof: &ProofFile,
    integrity: &EvidenceIntegrityResult,
) {
    ctx.heading("Cryptographic & Time Proofs");

    let registered = &proof.proof.root;
    let recomputed = integrity.recomputed_root.as_deref();
    let comparison = merkle_comparison(registered, recomputed);

    ctx.field("Merkle Root (as registered):", registered);
    match recomputed {
        Some(root) => ctx.field("Merkle Root (recomputed now):", root),
        None => ctx.field("Merkle Root (recomputed now):", "not recomputed"),
    }
    ctx.field("Merkle Root Comparison:", comparison);

    ctx.gap();
    ctx.field("Digital Signature (Ed25519):", &proof.proof.signature);
    ctx.field("Public Key:", &proof.proof.public_key);

    ctx.gap();
    ctx.field("TSA Status:", tsa_status_label(record.tsa_status));
    match record.tsa_status {
        TsaStatus::Confirmed => {
            if let Some(tsa) = proof.tsa.as_ref() {
                match tsa.timestamp {
                    Some(ts) => ctx.field("TSA Timestamp (unix):", &ts.to_string()),
                    None => ctx.field("TSA Timestamp (unix):", "(not provided)"),
                }
                match tsa.serial.as_deref() {
                    Some(s) => ctx.field("TSA Serial:", s),
                    None => ctx.field("TSA Serial:", "(not provided)"),
                }
                match tsa.token_bytes {
                    Some(n) => ctx.field("TSA Token Bytes:", &n.to_string()),
                    None => ctx.field("TSA Token Bytes:", "(not provided)"),
                }
            } else {
                ctx.line(
                    "TSA Status is CONFIRMED on the Evidence Record, but ProofFile.tsa is absent.",
                    9.5,
                );
            }
        }
        TsaStatus::Pending | TsaStatus::Absent | TsaStatus::Failed => {
            // Status already printed; no fictional PASS / invented provider.
        }
    }
}

fn write_verification_model(ctx: &mut PdfCtx) {
    ctx.heading("Verification Model");
    ctx.wrap_paragraph(VERIFICATION_MODEL);
}

fn write_scope_of_attestation(ctx: &mut PdfCtx) {
    ctx.heading("Scope of Attestation");
    ctx.wrap_paragraph(
        "This File Certificate is a projection over an Evident Ledger registration. \
         It attests that a content hash was recorded as a ledger event on a named chain, \
         that the parent-linked event structure and full-leaf merkle-root-v1 recomputation \
         were evaluated as reported in Integrity Verification, and that an Ed25519 \
         signature over (chain_id:merkle_root:chain_head) was checked when Signature Valid \
         is true.",
    );
    ctx.gap();
    ctx.wrap_paragraph(
        "This certificate does not attest the truthfulness of document contents, \
         legal validity in any jurisdiction, author identity beyond optional identity \
         layers outside this PDF, or that the original file is stored by Evident Ledger.",
    );
    ctx.gap();
    ctx.wrap_paragraph(
        "[N/A] Per-file Merkle inclusion proof (not implemented — full \
         leaf-set verification only, see Verification Model above).",
    );
}

fn write_independent_verification(ctx: &mut PdfCtx) {
    ctx.heading("Independent Verification");
    ctx.wrap_paragraph(
        "Planned CLI (not yet shipped in Stage 2.1 — Task 2.3): \
         evident verify --event <event_id> --chain <chain_id>",
    );
    ctx.gap();
    ctx.wrap_paragraph(
        "Public web verification at /cert/{certificate_id} is not available yet \
         (Stage 4). Do not treat QR or web URLs as working verifiers in this release.",
    );
    ctx.gap();
    ctx.wrap_paragraph(
        "Verification recomputes the Merkle root from the complete set of ledger \
         leaves for this chain — see Scope of Attestation.",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{EventLeaf, ProofPayload, TsaData};
    use crate::evidence_record::{
        build_registered_record, refresh_lifecycle, EvidenceFileMeta, LifecycleStatus,
    };
    use crate::merkle::MerkleTree;
    use crate::signing::ServerSigner;
    use std::fs;
    use std::process::Command;
    use tempfile::tempdir;
    use uuid::Uuid;

    /// Prefer `pdftotext` (same approach as `evident-report`); fall back to raw
    /// byte search for environments without poppler.
    fn pdf_text(bytes: &[u8]) -> String {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cert.pdf");
        fs::write(&path, bytes).unwrap();
        if let Ok(out) = Command::new("pdftotext").arg(&path).arg("-").output() {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout).into_owned();
            }
        }
        String::from_utf8_lossy(bytes).into_owned()
    }

    fn pdf_contains(bytes: &[u8], needle: &str) -> bool {
        pdf_text(bytes).contains(needle)
    }

    fn signed_fixture(
        local_available: bool,
        with_tsa: bool,
    ) -> (EvidenceRecord, ProofFile, String) {
        let dir = tempdir().unwrap();
        let key_path = dir.path().join("signing.key");
        let signer = ServerSigner::load_or_create(key_path.to_str().unwrap()).unwrap();

        let chain_id = Uuid::parse_str("c0bafd33-6807-4fb7-b480-c454ecabdd5d").unwrap();
        let event_id = Uuid::parse_str("22d29a6a-4cb4-469f-bbce-1d07e49694ce").unwrap();
        let file_hash = "d058b1ba7f8199b4b82f4c0861572d081004de5bd375e077c4eeb5641b725da0".to_string();
        let leaf = MerkleTree::build_leaf(1, &event_id, &Uuid::nil(), &file_hash);
        let root = MerkleTree::build_merkle_root(&[leaf]);
        let chain_head = event_id.to_string();
        let signature = signer.sign_root(&chain_id.to_string(), &root, &chain_head);

        let tsa = if with_tsa {
            Some(TsaData {
                timestamp: Some(1_786_372_690),
                serial: Some("tsr-1786372690".into()),
                token_bytes: Some(4642),
            })
        } else {
            None
        };

        let proof = ProofFile {
            leaf_version: "leaf_v1".into(),
            chain_id: chain_id.to_string(),
            head_event_id: chain_head.clone(),
            proof: ProofPayload {
                root: root.clone(),
                chain_head,
                signature,
                public_key: signer.public_key_hex(),
                leaves_count: 1,
                version: Some("proof_v1".into()),
                proof_type: Some("merkle-root-v1".into()),
            },
            events: vec![EventLeaf {
                sequence: 1,
                event_id: event_id.to_string(),
                parent_event_id: Uuid::nil().to_string(),
                file_hash: file_hash.clone(),
            }],
            tsa,
        };

        let record = build_registered_record(
            event_id,
            chain_id,
            &file_hash,
            Some(245_810),
            &EvidenceFileMeta {
                filename: Some("contract.pdf".into()),
                mime_type: Some("application/pdf".into()),
                local_file_available: local_available,
                project_id: Some("e2e".into()),
            },
            proof.tsa.as_ref(),
            None,
            Utc::now(),
        );

        (record, proof, root)
    }

    #[test]
    fn generates_pdf_for_tsa_confirmed() {
        let (record, proof, _) = signed_fixture(true, true);
        assert_eq!(record.lifecycle_status, LifecycleStatus::TsaConfirmed);
        let bytes = generate_file_certificate(&record, &proof).expect("pdf");
        assert!(!bytes.is_empty());
        assert!(bytes.starts_with(b"%PDF"));
        assert!(pdf_contains(&bytes, "TSA_CONFIRMED"));
        assert!(pdf_contains(&bytes, "Digital Signature (Ed25519)"));
    }

    #[test]
    fn generates_pdf_for_certified() {
        let (mut record, proof, _) = signed_fixture(true, true);
        refresh_lifecycle(&mut record, &proof);
        assert_eq!(record.lifecycle_status, LifecycleStatus::Certified);
        let bytes = generate_file_certificate(&record, &proof).expect("pdf");
        assert!(!bytes.is_empty());
        assert!(bytes.starts_with(b"%PDF"));
        assert!(pdf_contains(&bytes, "CERTIFIED"));
    }

    #[test]
    fn local_file_unavailable_is_labeled() {
        let (record, proof, _) = signed_fixture(false, true);
        let bytes = generate_file_certificate(&record, &proof).expect("pdf");
        assert!(pdf_contains(&bytes, "NOT AVAILABLE ON THIS DEVICE"));
    }

    #[test]
    fn merkle_roots_show_match_when_equal() {
        let (record, proof, root) = signed_fixture(true, true);
        let integrity = verify_evidence_integrity(&record, &proof);
        assert_eq!(integrity.recomputed_root.as_deref(), Some(root.as_str()));
        assert_eq!(
            merkle_comparison(&proof.proof.root, integrity.recomputed_root.as_deref()),
            "MATCH"
        );

        let bytes = generate_file_certificate(&record, &proof).expect("pdf");
        assert!(pdf_contains(&bytes, "Merkle Root (as registered)"));
        assert!(pdf_contains(&bytes, "Merkle Root (recomputed now)"));
        assert!(pdf_contains(&bytes, "MATCH"));
        assert!(pdf_contains(&bytes, &root));
    }

    #[test]
    fn merkle_comparison_not_available_when_recompute_fails() {
        let (record, mut proof, root) = signed_fixture(true, true);
        // Break parent chain so check_event_structure fails → recomputed_root = None.
        proof.events[0].parent_event_id = Uuid::new_v4().to_string();

        let integrity = verify_evidence_integrity(&record, &proof);
        assert!(integrity.recomputed_root.is_none());
        assert_eq!(
            merkle_comparison(&root, integrity.recomputed_root.as_deref()),
            "NOT AVAILABLE"
        );

        let bytes = generate_file_certificate(&record, &proof).expect("pdf");
        assert!(pdf_contains(&bytes, "not recomputed"));
        assert!(pdf_contains(&bytes, "NOT AVAILABLE"));
        // Must not claim MATCH when recompute failed.
        assert!(
            !pdf_contains(&bytes, "Merkle Root Comparison: MATCH"),
            "must not claim MATCH without recomputed root"
        );
    }
}
