//! Public Evidence Certificate PDF (Stage 6.4).
//!
//! Rendered only from `public_proof_id` — never from raw file hash.

use crate::public_proof::PublicRegistryEntry;
use printpdf::*;
use std::io::Cursor;

const PAGE_WIDTH: f32 = 210.0;
const PAGE_HEIGHT: f32 = 297.0;
const MARGIN_LEFT: f32 = 20.0;
const MARGIN_TOP: f32 = 25.0;
const MARGIN_BOTTOM: f32 = 20.0;
const LINE_HEIGHT: f32 = 6.0;

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

    fn line(&mut self, text: &str, size: f32) {
        if self.y - LINE_HEIGHT < MARGIN_BOTTOM {
            let (page, layer) = self.doc.add_page(Mm(PAGE_WIDTH), Mm(PAGE_HEIGHT), "Layer");
            self.layer = self.doc.get_page(page).get_layer(layer);
            self.y = PAGE_HEIGHT - MARGIN_TOP;
        }
        self.layer
            .use_text(text, size, Mm(MARGIN_LEFT), Mm(self.y), &self.font);
        self.y -= LINE_HEIGHT;
    }

    fn bold_line(&mut self, text: &str, size: f32) {
        if self.y - LINE_HEIGHT < MARGIN_BOTTOM {
            let (page, layer) = self.doc.add_page(Mm(PAGE_WIDTH), Mm(PAGE_HEIGHT), "Layer");
            self.layer = self.doc.get_page(page).get_layer(layer);
            self.y = PAGE_HEIGHT - MARGIN_TOP;
        }
        self.layer
            .use_text(text, size, Mm(MARGIN_LEFT), Mm(self.y), &self.bold);
        self.y -= LINE_HEIGHT;
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

    ctx.bold_line("PUBLIC EVIDENCE CERTIFICATE", 16.0);
    ctx.line("Status: REGISTERED", 11.0);
    ctx.line(&format!("Public Proof ID: {}", entry.public_proof_id), 10.0);
    ctx.line(
        &format!(
            "Registration Time: {} UTC",
            entry.registered_at.format("%Y-%m-%d %H:%M:%S")
        ),
        10.0,
    );
    ctx.line(&format!("TSA Class: {}", entry.tsa_class), 10.0);
    ctx.line(&format!("Integrity: {}", entry.integrity_state), 10.0);
    ctx.line("", 10.0);
    ctx.line("This certificate confirms that a file with this hash", 10.0);
    ctx.line("was registered in the Evident Ledger system.", 10.0);
    ctx.line("No additional metadata is disclosed.", 10.0);

    ctx.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn public_certificate_pdf_bytes_are_non_empty() {
        let entry = PublicRegistryEntry {
            public_proof_id: "pv_test123".to_string(),
            file_hash: "a".repeat(64),
            proof_status: "REGISTERED".to_string(),
            registered_at: Utc::now(),
            tsa_class: "legal".to_string(),
            integrity_state: "VALID".to_string(),
            enabled: true,
        };
        let bytes = render_public_certificate_pdf(&entry);
        assert!(bytes.starts_with(b"%PDF"));
    }
}
