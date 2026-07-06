use std::path::Path;
use std::fs::File;
use std::io::BufWriter;
use printpdf::*;
use chrono::Utc;

use crate::{ProofData, VerificationContext};

const LINE_HEIGHT: f32 = 20.0;

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

pub fn write_pdf(proof: &ProofData, verification: &VerificationContext, output_path: &Path) -> Result<()> {
    let (doc, page1, layer1) = PdfDocument::new(
        "Evident Ledger Proof Report",
        Mm(210.0),
        Mm(297.0),
        "Layer 1",
    );
    let layer = doc.get_page(page1).get_layer(layer1);
    let font = doc.add_builtin_font(BuiltinFont::Helvetica).unwrap();

    let mut y = 800.0;

    add_header(&layer, &font, proof, verification, &mut y);
    add_summary(&layer, &font, proof, verification, &mut y);
    add_events(&layer, &font, proof, &mut y);
    add_proof_block(&layer, &font, proof, &mut y);
    add_instructions(&layer, &font, &mut y);

    let file = File::create(output_path).map_err(|_| ReportError::IoError)?;
    doc.save(&mut BufWriter::new(file))
        .map_err(|_| ReportError::PdfGenerationFailed)?;

    Ok(())
}

fn add_header(layer: &PdfLayerReference, font: &IndirectFontRef, proof: &ProofData, verification: &VerificationContext, y: &mut f32) {
    let status_text = if verification.is_valid { "VALID" } else { "INVALID" };
    let trusted_timestamp_text = match proof.created_at {
        Some(ts) => ts.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        None => "UNANCHORED — no external timestamp evidence".to_string(),
    };
    let text = format!(
        "EVIDENT LEDGER PROOF REPORT\n\
         ─────────────────────────────\n\
         Chain ID: {}\n\
         Last Trusted Timestamp: {}\n\
         Verification Time: {}\n\
         Chain Status: {}",
        proof.chain_id,
        trusted_timestamp_text,
        verification.verified_at.format("%Y-%m-%d %H:%M:%S UTC"),
        status_text
    );
    layer.use_text(&text, 14.0, Mm(50.0), Mm(*y), font);
    *y -= LINE_HEIGHT * 6.0;
}

fn add_summary(layer: &PdfLayerReference, font: &IndirectFontRef, proof: &ProofData, verification: &VerificationContext, y: &mut f32) {
    let status_text = if verification.is_valid { "VALID" } else { "INVALID" };
    let mut text = format!(
        "SUMMARY\n\
         ────────\n\
         Chain ID:      {}\n\
         Head Event:    {}\n\
         Events:        {}\n\
         Merkle Root:   {}\n\
         Status:        {}",
        proof.chain_id,
        proof.head_event_id,
        proof.events.len(),
        proof.root,
        status_text
    );

    if !verification.is_valid {
        if let Some(seq) = verification.first_failure_sequence {
            text.push_str(&format!("\nFirst Failure Event: {}", seq));
        }
        if let Some(err) = &verification.first_failure_error {
            text.push_str(&format!("\nFirst Failure Error: {}", err));
        }
    }

    layer.use_text(&text, 11.0, Mm(50.0), Mm(*y), font);
    *y -= LINE_HEIGHT * 9.0;
}

fn add_events(layer: &PdfLayerReference, font: &IndirectFontRef, proof: &ProofData, y: &mut f32) {
    layer.use_text("EVENTS", 12.0, Mm(50.0), Mm(*y), font);
    *y -= LINE_HEIGHT;

    let mut table = String::new();
    table.push_str(&format!("{:>4} | {:36} | {}\n", "#", "Event ID", "File Hash"));
    table.push_str(&format!("{}", "-".repeat(80)));
    table.push('\n');

    for (i, event) in proof.events.iter().enumerate() {
        table.push_str(&format!(
            "{:>4} | {} | {}\n",
            i + 1,
            &event.event_id[..8],
           &event.file_hash.chars().take(16).collect::<String>()
        ));
    }

    layer.use_text(&table, 8.0, Mm(50.0), Mm(*y), font);
    *y -= LINE_HEIGHT * (proof.events.len() as f32 + 2.0);
}

fn add_proof_block(layer: &PdfLayerReference, font: &IndirectFontRef, proof: &ProofData, y: &mut f32) {
    let mut text = "PROOF BLOCK\n".to_string();
    text.push_str(&format!("─────────────\n"));
    text.push_str(&format!("Root:      {}\n", proof.root));
    text.push_str(&format!("Signature: {}\n", &proof.signature[..64]));
    text.push_str(&format!("Public Key: {}\n", &proof.public_key[..32]));

    match &proof.tsa {
        Some(tsa) => {
            text.push_str(&format!("\nTSA: {}\n", tsa.serial));
            text.push_str(&format!("Timestamp: {}\n", tsa.timestamp));
            text.push_str(&format!("Token Size: {} bytes", tsa.token_bytes));
        }
        None => {
            text.push_str("\nTSA Status: UNANCHORED\n");
            text.push_str("External timestamp evidence: not available");
        }
    }

    layer.use_text(&text, 9.0, Mm(50.0), Mm(*y), font);
    *y -= LINE_HEIGHT * 7.0;
}

fn add_instructions(layer: &PdfLayerReference, font: &IndirectFontRef, y: &mut f32) {
    let text = format!(
        "VERIFY INSTRUCTION\n\
         ──────────────────\n\
         To verify this proof offline:\n\
         \n\
         $ evident verify proof.json\n\
         \n\
         This proof is self-contained and can be verified\n\
         without server access."
    );
    layer.use_text(&text, 10.0, Mm(50.0), Mm(*y), font);
    *y -= LINE_HEIGHT * 8.0;
}
