use std::path::Path;
use std::fs::File;
use std::io::BufWriter;
use printpdf::*;
use chrono::Utc;

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
/// through this struct so no section can silently render off-page again.
struct Ctx {
    doc: PdfDocumentReference,
    layer: PdfLayerReference,
    font: IndirectFontRef,
    y: f32,
}

impl Ctx {
    fn new(doc: PdfDocumentReference, layer: PdfLayerReference, font: IndirectFontRef) -> Self {
        Self { doc, layer, font, y: PAGE_HEIGHT - MARGIN_TOP }
    }

    fn ensure_space(&mut self, lines_needed: f32) {
        let needed = LINE_HEIGHT * lines_needed;
        if self.y - needed < MARGIN_BOTTOM {
            let (page, layer) = self.doc.add_page(Mm(PAGE_WIDTH), Mm(PAGE_HEIGHT), "Layer");
            self.layer = self.doc.get_page(page).get_layer(layer);
            self.y = PAGE_HEIGHT - MARGIN_TOP;
        }
    }

    /// Raw line-by-line output, NO word-wrapping. Use this for pre-formatted
    /// content (tables) whose column widths are already fixed by the caller —
    /// running such content through wrap_text() breaks column alignment.
    fn raw_line(&mut self, text: &str, size: f32) {
        self.ensure_space(1.0);
        self.layer.use_text(text, size, Mm(MARGIN_LEFT), Mm(self.y), &self.font);
        self.y -= LINE_HEIGHT;
    }

    fn raw_block(&mut self, text: &str, size: f32) {
        for line in text.split('\n') {
            self.raw_line(line, size);
        }
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

    let mut ctx = Ctx::new(doc, layer, font);

    add_header(&mut ctx, proof, verification);
    add_events(&mut ctx, verification);
    add_proof_block(&mut ctx, proof);
    add_tsa_details_block(&mut ctx, proof);
    add_verification_scope(&mut ctx);
    add_instructions(&mut ctx);

    let doc = ctx.finish();
    let file = File::create(output_path).map_err(|_| ReportError::IoError)?;
    doc.save(&mut BufWriter::new(file))
        .map_err(|_| ReportError::PdfGenerationFailed)?;

    Ok(())
}

fn add_header(ctx: &mut Ctx, proof: &ProofData, verification: &VerificationContext) {
    let status_text = if verification.is_valid { "VALID" } else { "INVALID" };
    let (trusted_timestamp_text, external_tsa_note) = match proof.created_at {
        Some(ts) => (ts.format("%Y-%m-%d %H:%M:%S UTC").to_string(), None),
        None => (
            "Not Available".to_string(),
            Some("External TSA Evidence: No RFC3161 timestamp was attached to this ledger state."),
        ),
    };
    let covered_events_text = if proof.events.is_empty() {
        "none".to_string()
    } else {
        format!("1-{}", proof.events.len())
    };
    let tsa_note_line = match external_tsa_note {
        Some(note) => format!("\n{}", note),
        None => String::new(),
    };
    let mut text = format!(
        "EVIDENT LEDGER\n\
         Independent Evidence Verification Report\n\
         ─────────────────────────────\n\
         Chain Identifier: {}\n\
         \n\
         EVIDENCE SNAPSHOT\n\
         ─────────────────────────────\n\
         Last Trusted Timestamp: {}{}\n\
         Events Covered: {}\n\
         \n\
         CURRENT VERIFICATION\n\
         ─────────────────────────────\n\
         Verification Performed: {}\n\
         Ledger Integrity: {}",
        proof.chain_id,
        trusted_timestamp_text,
        tsa_note_line,
        covered_events_text,
        verification.verified_at.format("%Y-%m-%d %H:%M:%S UTC"),
        status_text
    );

    if !verification.is_valid {
        if let Some(seq) = verification.first_failure_sequence {
            text.push_str(&format!("\nFirst Integrity Failure: Event #{}", seq));
        }
        if let Some(err) = &verification.first_failure_error {
            text.push_str(&format!("\nFailure Reason: {}", err));
        }
    }

    ctx.wrapped_block(&text, 14.0);
    ctx.gap();
}

fn add_events(ctx: &mut Ctx, verification: &VerificationContext) {
    ctx.ensure_space(3.0);
    ctx.raw_line("REGISTERED EVIDENCE ITEMS", 12.0);

    let header = format!("{:>4} | {:30} | {:12} | {}", "#", "Evidence Item", "Chain Status", "Current File Integrity");
    ctx.raw_line(&header, 8.0);
    ctx.raw_line(&"-".repeat(80), 8.0);

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
        ctx.raw_line(&row, 8.0);
    }

    ctx.gap();
}

fn add_proof_block(ctx: &mut Ctx, proof: &ProofData) {
    let text = format!(
        "CRYPTOGRAPHIC PROOF\n\
         ────────────────────────────\n\
         Merkle Root: {}\n\
         Digital Signature: {}\n\
         Public Key Fingerprint: {}",
        proof.root,
        &proof.signature[..64],
        &proof.public_key[..32]
    );

    ctx.wrapped_block(&text, 9.0);
    ctx.gap();
}

fn add_tsa_details_block(ctx: &mut Ctx, proof: &ProofData) {
    let text = match &proof.tsa {
        Some(tsa) => format!(
            "EXTERNAL TIME ANCHOR DETAILS\n\
             ────────────────────────────\n\
             Provider: freetsa.org/tsr\n\
             Status: VALID\n\
             Timestamp: {}\n\
             Serial: {}\n\
             Token Size: {} bytes",
            tsa.timestamp,
            tsa.serial,
            tsa.token_bytes
        ),
        None => "EXTERNAL TIME ANCHOR DETAILS\n\
             ────────────────────────────\n\
             Status: UNANCHORED\n\
             External timestamp evidence: not available".to_string(),
    };

    ctx.wrapped_block(&text, 9.0);
    ctx.gap();
}

fn add_verification_scope(ctx: &mut Ctx) {
    let text = "VERIFICATION SCOPE\n\
         ────────────────────────────\n\
         This report confirms:\n\
         - The integrity of the registered ledger chain.\n\
         - The consistency of recorded evidence hashes.\n\
         - The validity of the cryptographic signature.\n\
         - The presence or absence of external timestamp evidence.\n\
         \n\
         This report does NOT confirm:\n\
         - Document authorship.\n\
         - Legal ownership.\n\
         - Document meaning or interpretation.\n\
         - Future immutability of external systems.";

    ctx.wrapped_block(text, 9.0);
    ctx.gap();
}

fn add_instructions(ctx: &mut Ctx) {
    let text = "OFFLINE VERIFICATION\n\
         ──────────────────\n\
         This evidence package can be independently verified using:\n\
         \n\
         $ evident verify proof.json\n\
         \n\
         This proof is self-contained and can be verified\n\
         without server access.";

    ctx.wrapped_block(text, 10.0);
}
