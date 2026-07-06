//! Deterministic renderer: SacDocument -> PDF bytes.
//! No DB, no HTTP, no TSA calls here — pure projection of an already-built document.
//! Layout works entirely in Mm; printpdf itself is the only mm/pt boundary.
//! printpdf 0.7's Mm wraps f32, so all layout arithmetic here is f32 to match.

use printpdf::*;
use std::io::Cursor;
use crate::sac::{SacDocument, SacTarget, SacVerificationStatus, SacTsaStatus};

pub const SAC_PDF_VERSION: &str = "1.0";

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
    fn new(doc: PdfDocumentReference, layer: PdfLayerReference, font: IndirectFontRef, bold: IndirectFontRef) -> Self {
        Self { doc, layer, font, bold, y: PAGE_HEIGHT - MARGIN_TOP }
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
        self.layer.use_text(text, size, Mm(MARGIN_LEFT), Mm(self.y), &self.font);
        self.y -= LINE_HEIGHT;
    }

    fn bold_line(&mut self, text: &str, size: f32) {
        self.ensure_space(1.0);
        self.layer.use_text(text, size, Mm(MARGIN_LEFT), Mm(self.y), &self.bold);
        self.y -= LINE_HEIGHT;
    }

    fn heading(&mut self, text: &str) {
        self.ensure_space(2.4);
        self.y -= SECTION_GAP;
        self.layer.use_text(text, 13.0, Mm(MARGIN_LEFT), Mm(self.y), &self.bold);
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
                .expect("PDF generation must not fail for a valid SacDocument");
        }
        buffer
    }
}

fn write_exclusions_section(ctx: &mut PdfCtx) {
    ctx.heading("Verification Scope");
    ctx.line("This certificate confirms:", 10.0);
    ctx.line("  - The existence (or absence) of a registered ledger state for the", 10.0);
    ctx.line("    Chain ID above.", 10.0);
    ctx.line("  - That the Merkle Root, if present, matches the record held by", 10.0);
    ctx.line("    the Evident Ledger at issuance time.", 10.0);
    ctx.line("  - That the accompanying signature, if present, is valid for the", 10.0);
    ctx.line("    stated public key.", 10.0);
    ctx.line("  - The presence or absence of an external RFC3161 timestamp.", 10.0);
    ctx.gap();
    ctx.line("This certificate does NOT confirm:", 10.0);
    ctx.line("  - The content of the underlying documents or events.", 10.0);
    ctx.line("  - The authorship of the underlying documents or events.", 10.0);
    ctx.line("  - That the ledger state described above will remain unchanged", 10.0);
    ctx.line("    in the future.", 10.0);
    ctx.gap();
    ctx.line("Not Verified:", 10.0);
    ctx.line("  - Document content", 10.0);
    ctx.line("  - Legal interpretation", 10.0);
    ctx.line("  - Authorship", 10.0);
}

fn write_footer_section(ctx: &mut PdfCtx, chain_id: &str) {
    ctx.ensure_space(6.0);
    ctx.gap();
    ctx.line("Verification performed by: Evident Ledger Verification Service", 9.0);
    ctx.line(&format!("Verify independently at:   /verify/{}/attestation", chain_id), 9.0);
    ctx.line(&format!("                            /verify/{}/attestation.pdf", chain_id), 9.0);
    ctx.gap();
    ctx.line("This certificate corresponds to the Merkle Root cited in the", 9.0);
    ctx.line("Evident Report for this Chain ID, if one has been issued.", 9.0);
}

pub fn render_sac_pdf(doc: &SacDocument) -> Vec<u8> {
    let (pdf_doc, page1, layer1) = PdfDocument::new(
        "Evident Ledger Independent Verification Certificate",
        Mm(PAGE_WIDTH),
        Mm(PAGE_HEIGHT),
        "Layer 1",
    );
    let font = pdf_doc.add_builtin_font(BuiltinFont::Helvetica).unwrap();
    let bold = pdf_doc.add_builtin_font(BuiltinFont::HelveticaBold).unwrap();
    let layer = pdf_doc.get_page(page1).get_layer(layer1);

    let mut ctx = PdfCtx::new(pdf_doc, layer, font, bold);

    let chain_id = match &doc.target {
        SacTarget::ChainId(id) => id.clone(),
        SacTarget::DocumentHash(h) => h.clone(),
    };

    ctx.bold_line("Evident Ledger Independent Verification Certificate", 16.0);
    ctx.line(&format!("SAC Certificate Format v{}", SAC_PDF_VERSION), 10.0);
    ctx.line(&format!("Chain ID: {}", chain_id), 11.0);
    ctx.line(&format!("Issued At: {}", doc.issued_at), 11.0);

    ctx.heading("Verification Result");

    if matches!(doc.verification.status, SacVerificationStatus::NotFound) {
        ctx.bold_line("Verification Result: NOT FOUND", 12.0);
        ctx.gap();
        ctx.line("This certificate attests that no ledger record exists for the", 10.0);
        ctx.line("requested Chain ID at the time of issuance.", 10.0);
        ctx.gap();
        ctx.line(&format!("Chain ID:   {}", chain_id), 10.0);
        ctx.line(&format!("Issued At:  {}", doc.issued_at), 10.0);
        ctx.gap();
        ctx.line("No further sections apply. See \"Verification Scope\" below.", 10.0);

        write_exclusions_section(&mut ctx);
        write_footer_section(&mut ctx, &chain_id);

        return ctx.finish();
    }

    match doc.verification.status {
        SacVerificationStatus::Verified => {
            ctx.bold_line("Verification Result: VERIFIED", 12.0);
        }
        SacVerificationStatus::Failed => {
            ctx.bold_line("Verification Result: FAILED", 12.0);
            if !doc.verification.errors.is_empty() {
                ctx.wrapped_field("Errors:", &doc.verification.errors.join("; "));
            }
        }
        SacVerificationStatus::NotFound => unreachable!(),
    }

    if let Some(state) = &doc.state {
        ctx.heading("Ledger State");
        ctx.wrapped_field("Merkle Root:", &state.merkle_root);
        ctx.wrapped_field("Head Event ID:", &state.head_event_id);
        ctx.wrapped_field("Last Event Timestamp:", &state.last_event_timestamp);
    }

    ctx.heading("Signature");
    ctx.wrapped_field(
        "Public Key Fingerprint:",
        doc.verification.public_key_fingerprint.as_deref().unwrap_or("N/A"),
    );
    ctx.wrapped_field(
        "Signature:",
        doc.verification.signature.as_deref().unwrap_or("N/A"),
    );

    ctx.heading("External Time Anchor (TSA)");
    match &doc.tsa {
        Some(tsa) if matches!(tsa.status, SacTsaStatus::Present) => {
            ctx.line("Status:     PRESENT", 10.0);
            ctx.wrapped_field("Provider:", tsa.provider.as_deref().unwrap_or("N/A"));
            ctx.line(&format!("Timestamp:  {}", tsa.timestamp.map(|t| t.to_string()).unwrap_or_default()), 10.0);
            ctx.wrapped_field("Serial:", tsa.serial.as_deref().unwrap_or("N/A"));
        }
        _ => {
            ctx.line("Status: NOT AVAILABLE", 10.0);
            ctx.gap();
            ctx.line("No external RFC3161 timestamp is attached to this record.", 10.0);
            ctx.line("This does not affect the validity of the ledger signature above.", 10.0);
        }
    }

    write_exclusions_section(&mut ctx);
    write_footer_section(&mut ctx, &chain_id);

    ctx.finish()
}
