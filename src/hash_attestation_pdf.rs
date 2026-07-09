//! Renderer: HashAttestationDocument -> PDF bytes.
//! Independent from sac_pdf.rs by design (frozen contract: separate
//! product artifact, no shared layout logic). Same pagination/wrapping
//! approach reused as plain code, not a shared abstraction, to keep the
//! two renderers decoupled per the architecture decision.

use crate::hash_attestation::HashAttestationDocument;
use printpdf::*;
use std::io::Cursor;

pub const HASH_ATTESTATION_PDF_VERSION: &str = "1.0";

const PAGE_WIDTH: f32 = 210.0;
const PAGE_HEIGHT: f32 = 297.0;
const MARGIN_LEFT: f32 = 20.0;
const MARGIN_TOP: f32 = 25.0;
const MARGIN_BOTTOM: f32 = 20.0;
const LINE_HEIGHT: f32 = 6.0;
const SECTION_GAP: f32 = 8.0;
const WRAP_CHARS: usize = 78;

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
            .use_text(text, 13.0, Mm(MARGIN_LEFT), Mm(self.y), &self.bold);
        self.y -= LINE_HEIGHT * 1.4;
    }

    fn gap(&mut self) {
        self.y -= SECTION_GAP;
    }

    fn wrapped_field(&mut self, label: &str, value: &str) {
        if label.len() + 1 + value.len() <= WRAP_CHARS {
            self.line(&format!("{} {}", label, value), 10.0);
            return;
        }
        self.line(label, 10.0);
        let chars: Vec<char> = value.chars().collect();
        for chunk in chars.chunks(WRAP_CHARS - 4) {
            let piece: String = chunk.iter().collect();
            self.line(&format!("    {}", piece), 10.0);
        }
    }

    fn finish(self) -> Vec<u8> {
        let mut buffer: Vec<u8> = Vec::new();
        {
            let mut writer = std::io::BufWriter::new(Cursor::new(&mut buffer));
            self.doc
                .save(&mut writer)
                .expect("PDF generation must not fail for a valid HashAttestationDocument");
        }
        buffer
    }
}

pub fn render_hash_attestation_pdf(doc: &HashAttestationDocument) -> Vec<u8> {
    let (pdf_doc, page1, layer1) = PdfDocument::new(
        "Hash Evidence Resolution Certificate",
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

    // 1. Header
    ctx.bold_line("Global Evidence Resolution Certificate", 16.0);
    ctx.line("Hash-Based Multi-Chain Attestation", 11.0);
    ctx.line(
        &format!("Hash Attestation Format Version: {}", doc.format_version),
        10.0,
    );
    ctx.gap();
    ctx.bold_line(
        &format!(
            "Resolution Status: {}",
            doc.resolution_status.replace('_', " ")
        ),
        12.0,
    );
    ctx.gap();
    ctx.wrapped_field("Hash:", &doc.hash);
    ctx.line(&format!("Issued At: {}", doc.issued_at), 10.0);
    ctx.wrapped_field("Request ID:", &doc.request_id);
    ctx.line(&format!("Matches Found: {}", doc.count), 11.0);

    // 2. Core Statement
    ctx.heading("Core Statement");
    ctx.line(
        "This document certifies that the provided hash has been observed",
        10.0,
    );
    ctx.line("within the Evident Ledger system.", 10.0);
    ctx.gap();
    ctx.line(
        "All matches below represent independent ledger chains where",
        10.0,
    );
    ctx.line("this hash was recorded.", 10.0);

    // 3. Matches (all, no filtering/ranking)
    ctx.heading("Matches");

    if doc.count == 0 {
        ctx.bold_line("NO OCCURRENCES FOUND", 12.0);
        ctx.line(
            "No ledger chain contains an event with this hash at the",
            10.0,
        );
        ctx.line("time of issuance.", 10.0);
    } else {
        for (i, m) in doc.matches.iter().enumerate() {
            ctx.ensure_space(9.0); // keep a single match block from splitting awkwardly where possible
            ctx.bold_line(&format!("--- MATCH #{} ---", i + 1), 11.0);
            ctx.wrapped_field("Chain ID:", &m.chain_id);
            ctx.wrapped_field("Event ID:", &m.event_id);
            ctx.line(&format!("Timestamp: {}", m.timestamp), 10.0);
            ctx.gap();
            ctx.wrapped_field("Merkle Root:", m.merkle_root.as_deref().unwrap_or("N/A"));
            ctx.wrapped_field(
                "Head Event ID:",
                m.head_event_id.as_deref().unwrap_or("N/A"),
            );
            ctx.gap();
            ctx.line(&format!("Verification: {}", m.verification_status), 10.0);
            ctx.line(&format!("TSA: {}", m.tsa_status), 10.0);
            ctx.gap();
        }
    }

    // 4. Interpretation Rule
    ctx.heading("Interpretation Rule");
    ctx.line(
        "Each match represents a valid occurrence of the same hash",
        10.0,
    );
    ctx.line("within an independent ledger chain.", 10.0);
    ctx.gap();
    ctx.line("No match overrides another.", 10.0);
    ctx.line("No ranking or prioritization is applied.", 10.0);
    ctx.line(
        "All records are equally authoritative within their own chain context.",
        10.0,
    );

    // 5. TSA Summary (aggregated only)
    let present = doc
        .matches
        .iter()
        .filter(|m| m.tsa_status == "PRESENT")
        .count();
    let missing = doc.count.saturating_sub(present);
    ctx.heading("External Time Anchors Summary");
    ctx.line(&format!("- Present: {}", present), 10.0);
    ctx.line(&format!("- Missing: {}", missing), 10.0);

    // 6. Footer
    ctx.gap();
    ctx.line(
        "This certificate is a deterministic aggregation of all ledger",
        9.0,
    );
    ctx.line("occurrences of the provided hash.", 9.0);
    ctx.gap();
    ctx.line(
        "It does not interpret intent, ownership, or legal meaning of the data.",
        9.0,
    );

    ctx.finish()
}
