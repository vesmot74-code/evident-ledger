use printpdf::*;
use std::fs::File;
use std::io::{BufWriter, Cursor};
use std::path::Path;

use crate::{ProofData, VerificationContext};

/// Loads DejaVu Sans (regular + bold) as embedded fonts. Base14 fonts
/// have no Cyrillic glyphs — evidence file names are frequently
/// Cyrillic, so a Base14-only renderer silently drops that text from
/// the evidence table. Reuses the font already vendored for notary-pdf.
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

// Fixed column x-offsets (mm, relative to MARGIN_LEFT) for the evidence
// table. Not monospace-dependent: DejaVu has no bundled monospace variant,
// and fixed x-coordinates per cell are correct regardless of glyph width
// anyway (padding-based alignment silently breaks under proportional or
// mixed-script text, which is exactly the bug this replaces).
const COL_NUM_X: f32 = 0.0;
const COL_NAME_X: f32 = 10.0;
const COL_CHAIN_X: f32 = 82.0;
const COL_INTEGRITY_X: f32 = 112.0;

const PAGE_WIDTH: f32 = 210.0;
const PAGE_HEIGHT: f32 = 297.0;
const MARGIN_LEFT: f32 = 50.0;
const MARGIN_TOP: f32 = 27.0;
const MARGIN_BOTTOM: f32 = 20.0;
const LINE_HEIGHT: f32 = 6.0;
const SECTION_GAP: f32 = 6.0;

fn color_navy() -> Rgb {
    Rgb::new(0.04, 0.14, 0.30, None)
}

fn color_pass() -> Rgb {
    Rgb::new(0.08, 0.50, 0.24, None)
}

fn color_fail() -> Rgb {
    Rgb::new(0.73, 0.11, 0.11, None)
}

fn color_gray_line() -> Rgb {
    Rgb::new(0.60, 0.60, 0.63, None)
}

fn color_header_bg() -> Rgb {
    Rgb::new(0.93, 0.93, 0.96, None)
}

fn color_black() -> Rgb {
    Rgb::new(0.0, 0.0, 0.0, None)
}

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

    /// Draws one evidence-table row using fixed per-column x-offsets
    /// instead of padded strings — correct regardless of glyph width,
    /// so Cyrillic file names align exactly like ASCII ones.
    fn table_row(
        &mut self,
        num: &str,
        name: &str,
        chain: &str,
        integrity: &str,
        size: f32,
        use_bold: bool,
    ) {
        self.ensure_space(1.0);
        let font = if use_bold { &self.bold } else { &self.font };
        self.layer
            .use_text(num, size, Mm(MARGIN_LEFT + COL_NUM_X), Mm(self.y), font);
        self.layer
            .use_text(name, size, Mm(MARGIN_LEFT + COL_NAME_X), Mm(self.y), font);
        self.layer
            .use_text(chain, size, Mm(MARGIN_LEFT + COL_CHAIN_X), Mm(self.y), font);
        self.layer.use_text(
            integrity,
            size,
            Mm(MARGIN_LEFT + COL_INTEGRITY_X),
            Mm(self.y),
            font,
        );
        self.y -= LINE_HEIGHT;
    }

    fn table_rule(&mut self) {
        self.ensure_space(1.0);
        let rule_width_mm = COL_INTEGRITY_X + 25.0;
        self.layer
            .use_text(&"-".repeat(1), 8.0, Mm(MARGIN_LEFT), Mm(self.y), &self.font);
        // Draw a simple horizontal line instead of a dash string: dash
        // width also depends on glyph metrics, same class of bug as the
        // old padded table. A line primitive is exact regardless of font.
        let line = Line {
            points: vec![
                (Point::new(Mm(MARGIN_LEFT), Mm(self.y + 2.0)), false),
                (
                    Point::new(Mm(MARGIN_LEFT + rule_width_mm), Mm(self.y + 2.0)),
                    false,
                ),
            ],
            is_closed: false,
        };
        self.layer.add_line(line);
        self.y -= LINE_HEIGHT;
    }

    fn set_fill(&mut self, rgb: Rgb) {
        self.layer.set_fill_color(Color::Rgb(rgb));
    }

    fn reset_fill(&mut self) {
        self.layer.set_fill_color(Color::Rgb(color_black()));
    }

    fn set_outline(&mut self, rgb: Rgb) {
        self.layer.set_outline_color(Color::Rgb(rgb));
    }

    fn colored_bold_line(&mut self, text: &str, size: f32, rgb: Rgb) {
        self.ensure_space(1.0);
        self.set_fill(rgb);
        self.layer
            .use_text(text, size, Mm(MARGIN_LEFT), Mm(self.y), &self.bold);
        self.reset_fill();
        self.y -= LINE_HEIGHT;
    }

    fn colored_rule(&mut self, rgb: Rgb, width_mm: f32) {
        self.ensure_space(0.5);
        self.set_outline(rgb);

        let line = Line {
            points: vec![
                (Point::new(Mm(MARGIN_LEFT), Mm(self.y + 3.0)), false),
                (
                    Point::new(Mm(MARGIN_LEFT + width_mm), Mm(self.y + 3.0)),
                    false,
                ),
            ],
            is_closed: false,
        };

        self.layer.add_line(line);
        self.set_outline(color_black());
    }

    fn table_header_bg(&mut self, width_mm: f32) {
        let top = self.y + 2.0;
        let bottom = self.y - LINE_HEIGHT + 1.0;

        self.set_fill(color_header_bg());

        self.layer.add_rect(Rect::new(
            Mm(MARGIN_LEFT - 2.0),
            Mm(bottom),
            Mm(MARGIN_LEFT + width_mm),
            Mm(top),
        ));

        self.reset_fill();
    }

    /// Roughly centers a bold line (Base14 metrics estimated at ~0.52em
    /// per character — sufficient for a certificate title).
    fn centered_bold_line(&mut self, text: &str, size: f32) {
        let approx_width_mm = text.chars().count() as f32 * size * 0.52 / MM_TO_PT;
        let x = ((PAGE_WIDTH - approx_width_mm) / 2.0).max(MARGIN_LEFT);
        self.ensure_space(1.0);
        self.layer
            .use_text(text, size, Mm(x), Mm(self.y), &self.bold);
        self.y -= LINE_HEIGHT;
    }

    fn heading(&mut self, text: &str) {
        self.ensure_space(2.2);
        self.gap();
        self.colored_bold_line(text, 12.0, color_navy());
        self.colored_rule(color_gray_line(), PAGE_WIDTH - MARGIN_LEFT - 20.0);
        self.y -= 2.0;
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

pub fn write_pdf(
    proof: &ProofData,
    verification: &VerificationContext,
    output_path: &Path,
) -> Result<()> {
    let (doc, page1, layer1) = PdfDocument::new(
        "Evident Ledger Proof Report",
        Mm(PAGE_WIDTH),
        Mm(PAGE_HEIGHT),
        "Layer 1",
    );
    let layer = doc.get_page(page1).get_layer(layer1);
    let (font, bold) = load_fonts(&doc);

    let mut ctx = Ctx::new(doc, layer, font, bold);

    add_header(&mut ctx, proof, verification);
    add_events(&mut ctx, verification);
    add_proof_block(&mut ctx, proof);
    add_tsa_details_block(&mut ctx, proof);
    add_verification_scope(&mut ctx, proof, verification);
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
    ctx.colored_rule(color_navy(), PAGE_WIDTH - MARGIN_LEFT - 20.0);
    ctx.gap();
    ctx.raw_line(&format!("Chain Identifier: {}", proof.chain_id), 10.0);

    if let Some(scope) = &proof.event_report_scope {
        add_event_report_scope_note(ctx, scope);
    }

    let (trusted_timestamp_text, external_tsa_note) = match proof.created_at {
        Some(ts) => (ts.format("%Y-%m-%d %H:%M:%S UTC").to_string(), None),
        None => (
            "Not Available".to_string(),
            Some("No RFC3161 timestamp was attached to this ledger state."),
        ),
    };

    ctx.heading("1. EVIDENCE SNAPSHOT");
    ctx.raw_line(
        &format!("Last Trusted Timestamp: {}", trusted_timestamp_text),
        10.0,
    );
    if let Some(note) = external_tsa_note {
        ctx.raw_line(note, 9.0);
    }

    if let Some(scope) = &proof.event_report_scope {
        // Event-level PDF: keep evidence presentation and crypto coverage separate.
        ctx.raw_line("Evidence Items Presented: 1", 10.0);
        ctx.raw_line(
            &format!(
                "Cryptographic Proof Scope: Events 1-{}",
                scope.proof_events_count
            ),
            10.0,
        );
    } else {
        let covered_events_text = if proof.events.is_empty() {
            "none".to_string()
        } else {
            format!("1-{}", proof.events.len())
        };
        ctx.raw_line(&format!("Events Covered: {}", covered_events_text), 10.0);
    }

    ctx.heading("2. CURRENT VERIFICATION");
    ctx.raw_line(
        &format!(
            "Verification Performed: {}",
            verification.verified_at.format("%Y-%m-%d %H:%M:%S UTC")
        ),
        10.0,
    );

    if verification.is_valid {
        ctx.colored_bold_line("[PASS] LEDGER INTEGRITY: VALID", 11.0, color_pass());
    } else {
        ctx.colored_bold_line("[FAIL] LEDGER INTEGRITY: INVALID", 11.0, color_fail());
        if let Some(seq) = verification.first_failure_sequence {
            ctx.raw_line(&format!("First Integrity Failure: Event #{}", seq), 10.0);
        }
        if let Some(err) = &verification.first_failure_error {
            ctx.wrapped_block(&format!("Failure Reason: {}", err), 9.0);
        }
    }
}

fn add_event_report_scope_note(ctx: &mut Ctx, scope: &crate::EventReportScope) {
    ctx.gap();
    ctx.heading("NOTE ON REPORT SCOPE");
    ctx.wrapped_block(
        "This report presents one registered evidence item from the ledger. \
         The cryptographic proof shown below represents the state of the \
         complete ledger chain at the time this report was generated. \
         It does not represent the historical chain state at the exact moment \
         the individual event was originally recorded.",
        9.0,
    );
    ctx.gap();
    ctx.raw_line(&format!("Evidence item: {}", scope.evidence_item_label), 9.0);
    ctx.raw_line(
        &format!(
            "Current chain head at report generation: {}",
            scope.chain_head_label
        ),
        9.0,
    );
    ctx.raw_line(
        &format!(
            "Events included in cryptographic proof: 1-{}",
            scope.proof_events_count
        ),
        9.0,
    );
    ctx.gap();
}

fn add_events(ctx: &mut Ctx, verification: &VerificationContext) {
    ctx.heading("3. REGISTERED EVIDENCE ITEMS");

    ctx.table_header_bg(137.0);

    ctx.table_row(
        "#",
        "Evidence Item",
        "Chain Status",
        "Original File Verification",
        8.0,
        true,
    );
    ctx.table_rule();

    for (i, file) in verification.files.iter().enumerate() {
        let chain_status = if file.chain_valid { "VALID" } else { "INVALID" };
        let local_status = match file.local_integrity_ok {
            Some(true) => "VALID",
            Some(false) => "TAMPERED",
            None => "NOT STORED",
        };
        let display_name: String = file.file_name.chars().take(36).collect();
        ctx.table_row(
            &format!("{}", i + 1),
            &display_name,
            chain_status,
            local_status,
            8.0,
            false,
        );
    }

    ctx.wrapped_block(
        "Note: Evident Ledger does not store original files. \"NOT STORED\" means no file \
         was presented for comparison at the time this report was generated — it does not \
         imply any issue with the registered evidence. \"VALID\" or \"TAMPERED\" indicates \
         the result of comparing a presented file's hash against the hash registered above.",
        7.5,
    );
}

fn add_proof_block(ctx: &mut Ctx, proof: &ProofData) {
    ctx.heading("4. CRYPTOGRAPHIC PROOF");
    ctx.wrapped_block(&format!("Merkle Root: {}", proof.root), 9.0);
    ctx.wrapped_block(
        &format!("Digital Signature: {}", &proof.signature[..64]),
        9.0,
    );
    ctx.wrapped_block(
        &format!("Public Key Fingerprint: {}", &proof.public_key[..32]),
        9.0,
    );
}

fn add_tsa_details_block(ctx: &mut Ctx, proof: &ProofData) {
    ctx.heading("5. TIME ATTESTATION");
    match &proof.tsa {
        Some(tsa) => {
            ctx.colored_bold_line(
                "[PASS] External TSA timestamp confirmed",
                10.0,
                color_pass(),
            );
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

/// Status labels for §6. Uses the same `verification.is_valid` as §2 for
/// chain / hashes / signature — no separate verification-model fields exist.
/// TSA presence follows §5 (`proof.tsa.is_some()`), not `is_valid`.
fn verification_scope_labels(is_valid: bool, tsa_present: bool) -> [&'static str; 4] {
    let pass_or_fail = if is_valid { "[PASS]" } else { "[FAIL]" };
    let tsa_status = if tsa_present { "[PASS]" } else { "[N/A]" };
    [pass_or_fail, pass_or_fail, pass_or_fail, tsa_status]
}

fn add_verification_scope(ctx: &mut Ctx, proof: &ProofData, verification: &VerificationContext) {
    // Path 4 (TZ Stage C): no granular fields on VerificationContext — bind the
    // three integrity lines to the same is_valid as §2; TSA to proof.tsa presence.
    let [chain_status, hashes_status, sig_status, tsa_status] =
        verification_scope_labels(verification.is_valid, proof.tsa.is_some());

    ctx.heading("6. VERIFICATION SCOPE");
    ctx.raw_line("This report confirms:", 9.0);
    ctx.gap();
    ctx.raw_line(
        &format!("{chain_status} Integrity of the registered ledger chain"),
        9.0,
    );
    ctx.raw_line(
        &format!("{hashes_status} Consistency of recorded evidence hashes"),
        9.0,
    );
    ctx.raw_line(
        &format!("{sig_status} Validity of the cryptographic signature"),
        9.0,
    );
    ctx.raw_line(
        &format!("{tsa_status} Presence or absence of external timestamp evidence"),
        9.0,
    );
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
    ctx.wrapped_block(
        "This evidence package can be independently verified using:",
        9.0,
    );
    ctx.gap();
    ctx.raw_line("$ evident verify proof.json", 9.0);
    ctx.gap();
    ctx.wrapped_block(
        "This proof is self-contained and can be verified without server access.",
        9.0,
    );
}

fn add_signature_block(ctx: &mut Ctx) {
    ctx.ensure_space(6.0);
    ctx.gap();
    ctx.raw_line("_________________________", 10.0);
    ctx.raw_line("Evident Ledger Client Utility", 9.0);
    ctx.gap();
    ctx.raw_line("_________________________", 10.0);
    ctx.raw_line(
        &format!(
            "Date: {}",
            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        ),
        9.0,
    );
}

#[cfg(test)]
mod tests {
    use super::verification_scope_labels;
    use crate::{
        generate_report, EventSummary, FileStatus, ProofData, TsaData, VerificationContext,
    };
    use chrono::Utc;
    use std::fs;

    fn sample_proof(with_tsa: bool) -> ProofData {
        ProofData {
            chain_id: "11111111-1111-1111-1111-111111111111".into(),
            head_event_id: "22222222-2222-2222-2222-222222222222".into(),
            events: vec![EventSummary {
                event_id: "22222222-2222-2222-2222-222222222222".into(),
                file_hash: "ab".repeat(32),
                sequence: Some(1),
            }],
            root: "cd".repeat(32),
            signature: "ef".repeat(64),
            public_key: "aa".repeat(32),
            tsa: with_tsa.then_some(TsaData {
                timestamp: 1_700_000_000,
                serial: "1".into(),
                token_bytes: 100,
            }),
            created_at: Some(Utc::now()),
            event_report_scope: None,
        }
    }

    fn sample_verification(is_valid: bool) -> VerificationContext {
        VerificationContext {
            is_valid,
            verified_at: Utc::now(),
            first_failure_sequence: if is_valid { None } else { Some(1) },
            first_failure_error: if is_valid {
                None
            } else {
                Some("test failure".into())
            },
            files: vec![FileStatus {
                file_name: "doc.txt".into(),
                chain_valid: is_valid,
                local_integrity_ok: None,
            }],
        }
    }

    #[test]
    fn scope_all_pass_when_valid_with_tsa() {
        let labels = verification_scope_labels(true, true);
        assert_eq!(labels, ["[PASS]", "[PASS]", "[PASS]", "[PASS]"]);
    }

    #[test]
    fn scope_integrity_fail_when_invalid_tsa_independent() {
        let with_tsa = verification_scope_labels(false, true);
        assert_eq!(with_tsa, ["[FAIL]", "[FAIL]", "[FAIL]", "[PASS]"]);

        let without_tsa = verification_scope_labels(false, false);
        assert_eq!(without_tsa, ["[FAIL]", "[FAIL]", "[FAIL]", "[N/A]"]);
    }

    #[test]
    fn scope_valid_without_tsa_uses_na_for_timestamp_line() {
        let labels = verification_scope_labels(true, false);
        assert_eq!(labels, ["[PASS]", "[PASS]", "[PASS]", "[N/A]"]);
    }

    #[test]
    fn generated_pdf_section2_and_scope_agree_when_invalid() {
        let dir = std::env::temp_dir().join("evident-report-scope");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(format!("invalid_scope-{}.pdf", std::process::id()));
        let proof = sample_proof(true);
        let verification = sample_verification(false);
        generate_report(&proof.chain_id, &proof, &verification, &path).expect("pdf");

        // Prefer pdftotext (literal UTF-8 may not appear in raw PDF bytes).
        let extracted = std::process::Command::new("pdftotext")
            .arg(&path)
            .arg("-")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());

        let text = match extracted {
            Some(t) => t,
            None => {
                // CI without poppler: label helper already covers Case 1/2.
                eprintln!("pdftotext unavailable; skipping PDF text assertions");
                return;
            }
        };

        assert!(
            text.contains("[FAIL] LEDGER INTEGRITY: INVALID"),
            "§2 must show INVALID"
        );
        assert!(
            text.contains("[FAIL] Integrity of the registered ledger chain"),
            "§6 chain line must FAIL when is_valid=false"
        );
        assert!(
            text.contains("[FAIL] Consistency of recorded evidence hashes"),
            "§6 hashes line must FAIL when is_valid=false"
        );
        assert!(
            text.contains("[FAIL] Validity of the cryptographic signature"),
            "§6 signature line must FAIL when is_valid=false"
        );
        assert!(
            text.contains("[PASS] Presence or absence of external timestamp evidence"),
            "§6 TSA line follows presence, not is_valid"
        );
        assert!(
            !text.contains("[PASS] Validity of the cryptographic signature"),
            "§6 must not keep static PASS on signature when invalid"
        );
    }

    #[test]
    fn event_level_pdf_separates_evidence_item_from_crypto_scope() {
        let dir = std::env::temp_dir().join("evident-report-event-scope");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(format!("event_scope-{}.pdf", std::process::id()));

        let mut proof = sample_proof(true);
        proof.root = "aa".repeat(32);
        proof.signature = "bb".repeat(64);
        proof.public_key = "cc".repeat(32);
        proof.event_report_scope = Some(crate::EventReportScope {
            evidence_item_label: "EVENT_001".into(),
            chain_head_label: "EVENT_003".into(),
            proof_events_count: 3,
        });
        let verification = sample_verification(true);
        generate_report(&proof.chain_id, &proof, &verification, &path).expect("pdf");

        let extracted = std::process::Command::new("pdftotext")
            .arg(&path)
            .arg("-")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        let Some(text) = extracted else {
            eprintln!("pdftotext unavailable; skipping PDF text assertions");
            return;
        };

        assert!(text.contains("NOTE ON REPORT SCOPE"));
        assert!(text.contains("Evidence item: EVENT_001"));
        assert!(text.contains("Current chain head at report generation: EVENT_003"));
        assert!(text.contains("Events included in cryptographic proof: 1-3"));
        assert!(text.contains("Evidence Items Presented: 1"));
        assert!(text.contains("Cryptographic Proof Scope: Events 1-3"));
        assert!(
            !text.contains("Events Covered:"),
            "event PDF must not use the combined Events Covered field"
        );
        // Crypto values unchanged (still from ProofData).
        assert!(text.contains(&format!("Merkle Root: {}", proof.root)));
        assert!(text.contains(&proof.signature[..64]));
        assert!(text.contains(&proof.public_key[..32]));
    }

    #[test]
    fn project_level_pdf_keeps_events_covered_field() {
        let dir = std::env::temp_dir().join("evident-report-project-scope");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(format!("project_scope-{}.pdf", std::process::id()));

        let mut proof = sample_proof(true);
        proof.events = vec![
            EventSummary {
                event_id: "a".into(),
                file_hash: "11".repeat(32),
                sequence: Some(1),
            },
            EventSummary {
                event_id: "b".into(),
                file_hash: "22".repeat(32),
                sequence: Some(2),
            },
            EventSummary {
                event_id: "c".into(),
                file_hash: "33".repeat(32),
                sequence: Some(3),
            },
        ];
        proof.event_report_scope = None;
        let verification = sample_verification(true);
        generate_report(&proof.chain_id, &proof, &verification, &path).expect("pdf");

        let extracted = std::process::Command::new("pdftotext")
            .arg(&path)
            .arg("-")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        let Some(text) = extracted else {
            eprintln!("pdftotext unavailable; skipping PDF text assertions");
            return;
        };

        assert!(text.contains("Events Covered: 1-3"));
        assert!(!text.contains("NOTE ON REPORT SCOPE"));
        assert!(!text.contains("Cryptographic Proof Scope:"));
        assert!(!text.contains("Evidence Items Presented:"));
    }
}
