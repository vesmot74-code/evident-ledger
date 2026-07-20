use printpdf::*;
use std::fs::File;
use std::io::{BufWriter, Cursor};
use std::path::Path;

use crate::pdf::ReportError;
use crate::ProofData;

const PAGE_WIDTH: f32 = 210.0;
const PAGE_HEIGHT: f32 = 297.0;
const MARGIN_LEFT: f32 = 50.0;
const MARGIN_TOP: f32 = 27.0;
const MARGIN_BOTTOM: f32 = 20.0;
const LINE_HEIGHT: f32 = 6.0;
const SECTION_GAP: f32 = 6.0;

fn load_fonts(doc: &PdfDocumentReference) -> (IndirectFontRef, IndirectFontRef) {
    let mut regular =
        Cursor::new(include_bytes!("../../vendor/notary-pdf/assets/fonts/DejaVuSans.ttf").as_ref());
    let mut bold = Cursor::new(
        include_bytes!("../../vendor/notary-pdf/assets/fonts/DejaVuSans-Bold.ttf").as_ref(),
    );
    let font = doc
        .add_external_font(&mut regular)
        .expect("load DejaVuSans.ttf");
    let font_bold = doc
        .add_external_font(&mut bold)
        .expect("load DejaVuSans-Bold.ttf");
    (font, font_bold)
}

struct Ctx {
    doc: PdfDocumentReference,
    layer: PdfLayerReference,
    font: IndirectFontRef,
    bold: IndirectFontRef,
    y: f32,
}

impl Ctx {
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

    fn raw_line(&mut self, text: &str, size: f32) {
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
        self.gap();
        self.bold_line(text, 11.0);
    }

    fn gap(&mut self) {
        self.y -= SECTION_GAP;
    }

    fn wrapped_block(&mut self, text: &str, size: f32) {
        const MAX_CHARS: usize = 88;
        for line in text.split('\n') {
            let mut current = String::new();
            for word in line.split(' ') {
                let candidate_len = if current.is_empty() {
                    word.chars().count()
                } else {
                    current.chars().count() + 1 + word.chars().count()
                };
                if candidate_len <= MAX_CHARS {
                    if !current.is_empty() {
                        current.push(' ');
                    }
                    current.push_str(word);
                } else {
                    if !current.is_empty() {
                        self.raw_line(&current, size);
                    }
                    current = word.to_string();
                }
            }
            if !current.is_empty() {
                self.raw_line(&current, size);
            }
        }
    }

    fn finish(self) -> PdfDocumentReference {
        self.doc
    }
}

/// Registration snapshot PDF — records what was fixed at commit time.
/// Not an independent verification report.
pub fn generate_registration_snapshot(proof: &ProofData, output_path: &Path) -> crate::Result<()> {
    if proof.events.is_empty() {
        return Err(ReportError::InvalidProofData);
    }

    let (doc, page1, layer1) = PdfDocument::new(
        "Evident Ledger Registration Snapshot",
        Mm(PAGE_WIDTH),
        Mm(PAGE_HEIGHT),
        "Layer 1",
    );
    let layer = doc.get_page(page1).get_layer(layer1);
    let (font, bold) = load_fonts(&doc);

    let mut ctx = Ctx::new(doc, layer, font, bold);

    ctx.bold_line("LEDGER REGISTRATION SNAPSHOT", 15.0);
    ctx.gap();
    ctx.raw_line(
        "This document records what the system registered at creation time.",
        9.0,
    );
    ctx.raw_line("It is not an independent verification report.", 9.0);
    ctx.gap();

    ctx.heading("1. LEDGER REGISTRATION");
    ctx.bold_line("Status: REGISTERED AT CREATION", 10.0);
    ctx.gap();
    ctx.raw_line("Information recorded at registration:", 9.0);
    ctx.raw_line("- Chain created.", 9.0);
    ctx.raw_line("- Hash calculated.", 9.0);
    ctx.raw_line("- Signature created.", 9.0);
    ctx.raw_line("- TSA state recorded.", 9.0);
    ctx.gap();
    ctx.raw_line(&format!("Chain Identifier: {}", proof.chain_id), 9.0);
    ctx.raw_line(&format!("Head Event ID: {}", proof.head_event_id), 9.0);
    if let Some(ts) = proof.created_at {
        ctx.raw_line(
            &format!(
                "Registration Time (UTC): {}",
                ts.format("%Y-%m-%d %H:%M:%S UTC")
            ),
            9.0,
        );
    }

    ctx.heading("2. REGISTERED EVIDENCE");
    for (i, event) in proof.events.iter().enumerate() {
        ctx.bold_line(
            &format!(
                "Event #{} — {}",
                event.sequence.unwrap_or(i as i64 + 1),
                event.event_id
            ),
            9.0,
        );
        ctx.raw_line(&format!("SHA-256: {}", event.file_hash), 8.5);
        ctx.gap();
    }

    ctx.heading("3. CRYPTOGRAPHIC PROOF");
    ctx.raw_line(&format!("Merkle Root: {}", proof.root), 9.0);
    ctx.raw_line(
        &format!(
            "Digital Signature: {}",
            &proof.signature[..64.min(proof.signature.len())]
        ),
        9.0,
    );
    ctx.raw_line(
        &format!(
            "Public Key Fingerprint: {}",
            &proof.public_key[..32.min(proof.public_key.len())]
        ),
        9.0,
    );

    ctx.heading("4. TIME ATTESTATION (RECORDED STATE)");
    match &proof.tsa {
        Some(tsa) => {
            ctx.raw_line("TSA state recorded at registration:", 9.0);
            ctx.raw_line("Provider: freetsa.org/tsr", 9.0);
            ctx.raw_line(&format!("Timestamp: {}", tsa.timestamp), 9.0);
            ctx.raw_line(&format!("Serial: {}", tsa.serial), 9.0);
            ctx.raw_line(&format!("Token Size: {} bytes", tsa.token_bytes), 9.0);
        }
        None => {
            ctx.raw_line("External TSA timestamp: not recorded at registration.", 9.0);
        }
    }

    ctx.heading("5. CURRENT FILE VERIFICATION");
    ctx.bold_line("Status: NOT PERFORMED", 10.0);
    ctx.gap();
    ctx.wrapped_block(
        "Reason: Registration snapshot generated immediately after commit. \
         No external file comparison was executed.",
        9.0,
    );
    ctx.gap();
    ctx.wrapped_block(
        "For independent verification of ledger integrity and optional file comparison, run:",
        9.0,
    );
    ctx.raw_line("$ evident verify proof.json", 9.0);

    ctx.gap();
    ctx.raw_line("_________________________", 10.0);
    ctx.raw_line("Evident Ledger Client Utility", 9.0);
    ctx.raw_line(
        &format!(
            "Date: {}",
            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        ),
        9.0,
    );

    let doc = ctx.finish();
    let file = File::create(output_path).map_err(|_| ReportError::IoError)?;
    doc.save(&mut BufWriter::new(file))
        .map_err(|_| ReportError::PdfGenerationFailed)?;

    Ok(())
}
