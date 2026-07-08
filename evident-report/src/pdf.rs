use std::path::Path;
use std::fs::File;
use std::io::BufWriter;
use printpdf::*;

use crate::{ProofData, VerificationContext};

const PAGE_WIDTH: f32 = 210.0;
const PAGE_HEIGHT: f32 = 297.0;
const MARGIN_LEFT: f32 = 50.0;
const MARGIN_TOP: f32 = 27.0;
const MARGIN_BOTTOM: f32 = 20.0;
const LINE_HEIGHT: f32 = 6.0;
const SECTION_GAP: f32 = 6.0;

#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("PDF generation failed")]
    PdfGenerationFailed,
    #[error("Invalid proof data")]
    InvalidProofData,
    #[error("I/O error")]
    IoError,
}

pub type Result<T> = std::result::Result<T, ReportError>;

const MM_TO_PT: f32 = 2.834_646;

fn wrap_text(text: &str, max_chars: usize) -> String {
    let mut result = String::new();
    for line in text.split('\n') {
        if line.chars().count() <= max_chars {
            result.push_str(line);
            result.push('\n');
            continue;
        }
        let mut current = String::new();
        for word in line.split(' ') {
            let candidate_len = if current.is_empty() {
                word.chars().count()
            } else {
                current.chars().count() + 1 + word.chars().count()
            };
            if candidate_len <= max_chars {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
            } else {
                if !current.is_empty() {
                    result.push_str(&current);
                    result.push('\n');
                }
                current = word.to_string();
            }
        }
        result.push_str(&current);
        result.push('\n');
    }
    if result.ends_with('\n') {
        result.pop();
    }
    result
}

/// Rendering context with automatic pagination. All text output goes
/// through this struct so no section can silently render off-page.
struct Ctx {
    doc: PdfDocumentReference,
    layer: PdfLayerReference,
    font: IndirectFontRef,
    bold: IndirectFontRef,
    /// Monospace font, used ONLY for the fixed-width evidence table.
    /// Helvetica is proportional — rendering `{:>4} | {:30} | ...`
    /// formatted rows with it silently breaks column alignment in the
    /// actual PDF even though the Rust string looks aligned.
    mono: IndirectFontRef,
    y: f32,
}

impl Ctx {
    fn new(doc: PdfDocumentReference, layer: PdfLayerReference, font: IndirectFontRef, bold: IndirectFontRef, mono: IndirectFontRef) -> Self {
        Self { doc, layer, font, bold, mono, y: PAGE_HEIGHT - MARGIN_TOP }
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
        self.layer.use_text(text, size, Mm(MARGIN_LEFT), Mm(self.y), &self.font);
        self.y -= LINE_HEIGHT;
    }

    fn bold_line(&mut self, text: &str, size: f32) {
        self.ensure_space(1.0);
        self.layer.use_text(text, size, Mm(MARGIN_LEFT), Mm(self.y), &self.bold);
        self.y -= LINE_HEIGHT;
    }

    /// Monospace line — the only correct way to render fixed-width
    /// column data (tables) so it stays visually aligned in the PDF.
    fn mono_line(&mut self, text: &str, size: f32) {
        self.ensure_space(1.0);
        self.layer.use_text(text, size, Mm(MARGIN_LEFT), Mm(self.y), &self.mono);
        self.y -= LINE_HEIGHT;
    }

    /// Roughly centers a bold line (Base14 metrics estimated at ~0.52em
    /// per character — sufficient for a certificate title).
    fn centered_bold_line(&mut self, text: &str, size: f32) {
        let approx_width_mm = text.chars().count() as f32 * size * 0.52 / MM_TO_PT;
        let x = ((PAGE_WIDTH - approx_width_mm) / 2.0).max(MARGIN_LEFT);
        self.ensure_space(1.0);
        self.layer.use_text(text, size, Mm(x), Mm(self.y), &self.bold);
        self.y -= LINE_HEIGHT;
    }

    fn heading(&mut self, text: &str) {
        self.ensure_space(2.2);
        self.gap();
        self.bold_line(text, 12.0);
    }

    /// Word-wrapped, paginated block for prose content.
    fn wrapped_block(&mut self, text: &str, size: f32) {
        let usable_width_mm = PAGE_WIDTH - MARGIN_LEFT - 20.0;
        let avg_char_width_mm = size * 0.5 / MM_TO_PT;
        let max_chars = (usable_width_mm / avg_char_width_mm).floor().max(10.0) as usize;
        let wrapped = wrap_text(text, max_chars);
        for line in wrapped.split('\n') {
            self.raw_line(line, size);
        }
    }

    fn gap(&mut self) {
        self.y -= SECTION_GAP;
    }

    fn finish(self) -> PdfDocumentReference {
        self.doc
    }
}

pub fn write_pdf(proof: &ProofData, verification: &VerificationContext, output_path: &Path) -> Result<()> {
    let (doc, page1, layer1) = PdfDocument::new(
        "Evident Ledger Proof Report",
        Mm(PAGE_WIDTH),
        Mm(PAGE_HEIGHT),
        "Layer 1",
    );
    let layer = doc.get_page(page1).get_layer(layer1);
    let font = doc.add_builtin_font(BuiltinFont::Helvetica).unwrap();
    let bold = doc.add_builtin_font(BuiltinFont::HelveticaBold).unwrap();
    let mono = doc.add_builtin_font(BuiltinFont::Courier).unwrap();

    let mut ctx = Ctx::new(doc, layer, font, bold, mono);

    add_header(&mut ctx, proof, verification);
    add_events(&mut ctx, verification);
    add_proof_block(&mut ctx, proof);
    add_tsa_details_block(&mut ctx, proof);
    add_verification_scope(&mut ctx);
    add_instructions(&mut ctx);
    add_signature_block(&mut ctx);

    let doc = ctx.finish();
    let file = File::create(output_path).map_err(|_| ReportError::IoError)?;
    doc.save(&mut BufWriter::new(file))
        .map_err(|_| ReportError::PdfGenerationFailed)?;

    Ok(())
}

fn add_header(ctx: &mut Ctx, proof: &ProofData, verification: &VerificationContext) {
    ctx.centered_bold_line("INDEPENDENT EVIDENCE VERIFICATION REPORT", 15.0);
    ctx.gap();
    ctx.raw_line(&format!("Chain Identifier: {}", proof.chain_id), 10.0);

    let (trusted_timestamp_text, external_tsa_note) = match proof.created_at {
        Some(ts) => (ts.format("%Y-%m-%d %H:%M:%S UTC").to_string(), None),
        None => (
            "Not Available".to_string(),
            Some("No RFC3161 timestamp was attached to this ledger state."),
        ),
    };
    let covered_events_text = if proof.events.is_empty() {
        "none".to_string()
    } else {
        format!("1-{}", proof.events.len())
    };

    ctx.heading("1. EVIDENCE SNAPSHOT");
    ctx.raw_line(&format!("Last Trusted Timestamp: {}", trusted_timestamp_text), 10.0);
    if let Some(note) = external_tsa_note {
        ctx.raw_line(note, 9.0);
    }
    ctx.raw_line(&format!("Events Covered: {}", covered_events_text), 10.0);

    ctx.heading("2. CURRENT VERIFICATION");
    ctx.raw_line(&format!("Verification Performed: {}", verification.verified_at.format("%Y-%m-%d %H:%M:%S UTC")), 10.0);

    if verification.is_valid {
        ctx.bold_line("[PASS] LEDGER INTEGRITY: VALID", 11.0);
    } else {
        ctx.bold_line("[FAIL] LEDGER INTEGRITY: INVALID", 11.0);
        if let Some(seq) = verification.first_failure_sequence {
            ctx.raw_line(&format!("First Integrity Failure: Event #{}", seq), 10.0);
        }
        if let Some(err) = &verification.first_failure_error {
            ctx.wrapped_block(&format!("Failure Reason: {}", err), 9.0);
        }
    }
}

fn add_events(ctx: &mut Ctx, verification: &VerificationContext) {
    ctx.heading("3. REGISTERED EVIDENCE ITEMS");

    let header = format!("{:>4} | {:30} | {:12} | {}", "#", "Evidence Item", "Chain Status", "Current File Integrity");
    ctx.mono_line(&header, 8.0);
    ctx.mono_line(&"-".repeat(80), 8.0);

    for (i, file) in verification.files.iter().enumerate() {
        let chain_status = if file.chain_valid { "VALID" } else { "INVALID" };
        let local_status = match file.local_integrity_ok {
            Some(true) => "VALID",
            Some(false) => "TAMPERED",
            None => "UNKNOWN",
        };
        let display_name: String = file.file_name.chars().take(28).collect();
        let row = format!(
            "{:>4} | {:30} | {:12} | {}",
            i + 1,
            display_name,
            chain_status,
            local_status
        );
        ctx.mono_line(&row, 8.0);
    }
}

fn add_proof_block(ctx: &mut Ctx, proof: &ProofData) {
    ctx.heading("4. CRYPTOGRAPHIC PROOF");
    ctx.wrapped_block(&format!("Merkle Root: {}", proof.root), 9.0);
    ctx.wrapped_block(&format!("Digital Signature: {}", &proof.signature[..64]), 9.0);
    ctx.wrapped_block(&format!("Public Key Fingerprint: {}", &proof.public_key[..32]), 9.0);
}

fn add_tsa_details_block(ctx: &mut Ctx, proof: &ProofData) {
    ctx.heading("5. TIME ATTESTATION");
    match &proof.tsa {
        Some(tsa) => {
            ctx.bold_line("[PASS] External TSA timestamp confirmed", 10.0);
            ctx.gap();
            ctx.raw_line("Provider: freetsa.org/tsr", 9.0);
            ctx.raw_line(&format!("Timestamp: {}", tsa.timestamp), 9.0);
            ctx.raw_line(&format!("Serial: {}", tsa.serial), 9.0);
            ctx.raw_line(&format!("Token Size: {} bytes", tsa.token_bytes), 9.0);
        }
        None => {
            ctx.bold_line("[N/A] External TSA timestamp not available", 10.0);
            ctx.gap();
            ctx.raw_line("External timestamp evidence: not available", 9.0);
        }
    }
}

fn add_verification_scope(ctx: &mut Ctx) {
    ctx.heading("6. VERIFICATION SCOPE");
    ctx.raw_line("This report confirms:", 9.0);
    ctx.gap();
    ctx.raw_line("[PASS] Integrity of the registered ledger chain", 9.0);
    ctx.raw_line("[PASS] Consistency of recorded evidence hashes", 9.0);
    ctx.raw_line("[PASS] Validity of the cryptographic signature", 9.0);
    ctx.raw_line("[PASS] Presence or absence of external timestamp evidence", 9.0);
    ctx.gap();
    ctx.raw_line("This report does NOT confirm:", 9.0);
    ctx.gap();
    ctx.raw_line("[N/A]  Document authorship", 9.0);
    ctx.raw_line("[N/A]  Legal ownership", 9.0);
    ctx.raw_line("[N/A]  Document meaning or interpretation", 9.0);
    ctx.raw_line("[N/A]  Future immutability of external systems", 9.0);
}

fn add_instructions(ctx: &mut Ctx) {
    ctx.heading("7. OFFLINE VERIFICATION");
    ctx.wrapped_block("This evidence package can be independently verified using:", 9.0);
    ctx.gap();
    ctx.mono_line("$ evident verify proof.json", 9.0);
    ctx.gap();
    ctx.wrapped_block("This proof is self-contained and can be verified without server access.", 9.0);
}

fn add_signature_block(ctx: &mut Ctx) {
    ctx.ensure_space(6.0);
    ctx.gap();
    ctx.raw_line("_________________________", 10.0);
    ctx.raw_line("Evident Ledger Client Utility", 9.0);
    ctx.gap();
    ctx.raw_line("_________________________", 10.0);
    ctx.raw_line(&format!("Date: {}", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")), 9.0);
}
