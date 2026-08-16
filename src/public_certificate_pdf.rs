//! Public Evidence Certificate PDF (Stage 6.4).
//!
//! Rendered only from `public_proof_id` — never from raw file hash.
//! Tier 1 disclosure is intentional and minimal (privacy-by-design).

use crate::public_proof::PublicRegistryEntry;
use printpdf::*;
use std::io::Cursor;

const PAGE_WIDTH: f32 = 210.0;
const PAGE_HEIGHT: f32 = 297.0;
const MARGIN_LEFT: f32 = 20.0;
const MARGIN_TOP: f32 = 25.0;
const MARGIN_BOTTOM: f32 = 20.0;
const LINE_HEIGHT: f32 = 6.0;
const WRAP_CHARS: usize = 78;

/// Canonical public certificate URL (same path used by File Certificate QR).
const PUBLIC_CERTIFICATE_BASE_URL: &str = "https://evident-ledger.com/public/verify";

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

    fn ensure_space(&mut self) {
        if self.y - LINE_HEIGHT < MARGIN_BOTTOM {
            let (page, layer) = self.doc.add_page(Mm(PAGE_WIDTH), Mm(PAGE_HEIGHT), "Layer");
            self.layer = self.doc.get_page(page).get_layer(layer);
            self.y = PAGE_HEIGHT - MARGIN_TOP;
        }
    }

    fn line(&mut self, text: &str, size: f32) {
        self.ensure_space();
        self.layer
            .use_text(text, size, Mm(MARGIN_LEFT), Mm(self.y), &self.font);
        self.y -= LINE_HEIGHT;
    }

    fn bold_line(&mut self, text: &str, size: f32) {
        self.ensure_space();
        self.layer
            .use_text(text, size, Mm(MARGIN_LEFT), Mm(self.y), &self.bold);
        self.y -= LINE_HEIGHT;
    }

    fn gap(&mut self) {
        self.y -= LINE_HEIGHT * 0.6;
    }

    fn wrap_paragraph(&mut self, text: &str, size: f32) {
        let mut current = String::new();
        for word in text.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.len() + 1 + word.len() <= WRAP_CHARS {
                current.push(' ');
                current.push_str(word);
            } else {
                self.line(&current, size);
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            self.line(&current, size);
        }
    }

    fn finish(self) -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut writer = std::io::BufWriter::new(Cursor::new(&mut buffer));
            self.doc
                .save(&mut writer)
                .expect("PDF generation must not fail for a valid public certificate");
        }
        buffer
    }
}

fn public_certificate_url(public_proof_id: &str) -> String {
    format!("{PUBLIC_CERTIFICATE_BASE_URL}/{public_proof_id}/certificate.pdf")
}

pub fn render_public_certificate_pdf(entry: &PublicRegistryEntry) -> Vec<u8> {
    let (pdf_doc, page1, layer1) = PdfDocument::new(
        "Public Evidence Certificate",
        Mm(PAGE_WIDTH),
        Mm(PAGE_HEIGHT),
        "Layer 1",
    );
    let font = pdf_doc.add_builtin_font(BuiltinFont::Helvetica).unwrap();
    let bold = pdf_doc
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .unwrap();
    let layer = pdf_doc.get_page(page1).get_layer(layer1);
    let mut ctx = PdfCtx::new(pdf_doc, layer, font, bold);

    // --- Title ---
    ctx.bold_line("PUBLIC EVIDENCE CERTIFICATE", 16.0);
    ctx.line("Evident Ledger - Tier 1 public verification summary", 9.5);
    ctx.gap();

    // --- Existing disclosure fields only (no new data) ---
    ctx.bold_line("Registration Summary", 12.0);
    ctx.line(&format!("Status: {}", entry.proof_status), 10.0);
    ctx.line(
        &format!("Public Proof ID: {}", entry.public_proof_id),
        10.0,
    );
    ctx.line(
        &format!(
            "Registration Time: {} UTC",
            entry.registered_at.format("%Y-%m-%d %H:%M:%S")
        ),
        10.0,
    );
    ctx.line(&format!("TSA Class: {}", entry.tsa_class), 10.0);
    ctx.line(&format!("Integrity: {}", entry.integrity_state), 10.0);
    ctx.gap();

    // --- What this certificate confirms ---
    ctx.bold_line("What This Certificate Confirms", 12.0);
    ctx.wrap_paragraph(
        "This certificate confirms that a file content hash was registered \
         in Evident Ledger and that a corresponding public registration \
         record is available for independent checking.",
        10.0,
    );
    ctx.gap();

    // --- Verification Scope (claims limited to disclosed fields) ---
    ctx.bold_line("Verification Scope", 12.0);
    ctx.line("This certificate confirms:", 10.0);
    ctx.line(
        "[PASS] The file registration exists in Evident Ledger.",
        10.0,
    );
    if entry.integrity_state.eq_ignore_ascii_case("VALID") {
        ctx.line(
            "[PASS] The registration record is integrity-protected.",
            10.0,
        );
    } else {
        ctx.wrap_paragraph(
            &format!(
                "[PASS] The public integrity state for this registration is \
                 reported as {}.",
                entry.integrity_state
            ),
            10.0,
        );
    }
    ctx.wrap_paragraph(
        "This public certificate summarizes the registration record above. \
         It does not reproduce the full cryptographic evidence chain.",
        9.5,
    );
    ctx.gap();

    // --- Privacy boundary ---
    ctx.bold_line("Privacy Boundary", 12.0);
    ctx.wrap_paragraph(
        "Additional chain and cryptographic metadata is not disclosed in \
         this public certificate in order to protect the confidentiality of \
         the underlying evidence chain.",
        10.0,
    );
    ctx.gap();

    // --- Public verification (URL derived from already-disclosed public_proof_id) ---
    ctx.bold_line("Public Verification", 12.0);
    ctx.line("Public verification:", 10.0);
    ctx.wrap_paragraph(&public_certificate_url(&entry.public_proof_id), 9.0);
    ctx.gap();

    // --- Closing (no Tier 2 / package claim) ---
    ctx.bold_line("Important", 12.0);
    ctx.wrap_paragraph(
        "This certificate provides a public verification summary. It does \
         not attest authorship, ownership, legal validity of document \
         contents, or the truthfulness of information in the underlying file.",
        9.5,
    );

    ctx.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::fs;
    use std::process::Command;
    use tempfile::tempdir;

    fn sample_entry() -> PublicRegistryEntry {
        PublicRegistryEntry {
            public_proof_id: "pv_test123".to_string(),
            file_hash: "a".repeat(64),
            proof_status: "REGISTERED".to_string(),
            registered_at: Utc::now(),
            tsa_class: "legal".to_string(),
            integrity_state: "VALID".to_string(),
            enabled: true,
        }
    }

    /// Prefer `pdftotext` — printpdf may compress content streams.
    fn pdf_text(bytes: &[u8]) -> String {
        let dir = tempdir().unwrap();
        let path = dir.path().join("public-cert.pdf");
        fs::write(&path, bytes).unwrap();
        if let Ok(out) = Command::new("pdftotext").arg(&path).arg("-").output() {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout).into_owned();
            }
        }
        String::from_utf8_lossy(bytes).into_owned()
    }

    #[test]
    fn public_certificate_pdf_bytes_are_non_empty() {
        let bytes = render_public_certificate_pdf(&sample_entry());
        assert!(bytes.starts_with(b"%PDF"));
    }

    #[test]
    fn public_certificate_explains_scope_and_privacy_without_new_disclosure() {
        let entry = sample_entry();
        let text = pdf_text(&render_public_certificate_pdf(&entry));

        assert!(text.contains("PUBLIC EVIDENCE CERTIFICATE"));
        assert!(text.contains("Status: REGISTERED"));
        assert!(text.contains("Public Proof ID: pv_test123"));
        assert!(text.contains("TSA Class: legal"));
        assert!(text.contains("Integrity: VALID"));
        assert!(text.contains("Verification Scope"));
        assert!(text.contains("[PASS] The file registration exists in Evident Ledger."));
        assert!(text.contains("[PASS] The registration record is integrity-protected."));
        assert!(text.contains("Privacy Boundary"));
        assert!(text.contains("protect the confidentiality"));
        assert!(text.contains("Public verification:"));
        assert!(text.contains(&public_certificate_url(&entry.public_proof_id)));

        // Disclosure must not expand beyond Tier 1 public fields.
        for forbidden in [
            "Chain ID:",
            "Event ID:",
            "merkle_root",
            "chain_id",
            "event_id",
            "Digital Signature",
            "Public Key",
            "SHA-256",
            entry.file_hash.as_str(),
            "Advanced Evidence Package",
            "Tier 2",
        ] {
            assert!(
                !text.contains(forbidden),
                "must not disclose or claim: {forbidden}"
            );
        }
    }
}
