use chrono::Local;
use printpdf::*;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;

const MAX_CHARS_PER_LINE: usize = 85;

// X positions (mm) for the three table columns
const TABLE_COL1_X: f32 = 25.0;
const TABLE_COL2_X: f32 = 90.0;
const TABLE_COL3_X: f32 = 135.0;

fn main() {
    let input = "docs/whitepaper/Evident_Ledger_Technical_Whitepaper_v1.0.md";
    let output = "docs/whitepaper/Evident_Ledger_Technical_Whitepaper_v1.0.pdf";

    println!("Evident Whitepaper Generator");
    println!("Input: {}", input);

    let content = fs::read_to_string(input).expect("Cannot read whitepaper markdown file");

    generate_pdf(&content, output);

    println!("PDF generated:");
    println!("{}", output);
}

fn wrap_line(line: &str, max_chars: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();

    for word in line.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            result.push(current.clone());
            current = word.to_string();
        }
    }

    if !current.is_empty() {
        result.push(current);
    }

    if result.is_empty() {
        result.push(String::new());
    }

    result
}

// Returns Some(cells) if this line is a markdown table row (| a | b | c |)
fn parse_table_row(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 1 {
        let cells: Vec<String> = trimmed[1..trimmed.len() - 1]
            .split('|')
            .map(|c| c.trim().to_string())
            .collect();
        Some(cells)
    } else {
        None
    }
}

// Returns true if the row is a markdown separator row like | --- | --- | --- |
fn is_separator_row(cells: &[String]) -> bool {
    cells
        .iter()
        .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'))
}

fn generate_pdf(text: &str, output: &str) {
    let (doc, page1, layer1) = PdfDocument::new(
        "Evident Ledger Technical Whitepaper",
        Mm(210.0),
        Mm(297.0),
        "Cover",
    );

    let doc = doc
        .with_title("Evident Ledger Technical Whitepaper")
        .with_author("Evident Ledger Project")
        .with_subject("Cryptographic Audit Infrastructure")
        .with_keywords(vec![
            "SHA-256",
            "Ed25519",
            "Merkle Tree",
            "TSA",
            "Evidence Verification",
        ]);

    let font = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .expect("Cannot load font");

    let font_bold = doc
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .expect("Cannot load bold font");

    let current_date = Local::now().format("%B %Y").to_string();

    //
    // COVER PAGE
    //

    let cover = doc.get_page(page1).get_layer(layer1);

    cover.use_text("EVIDENT LEDGER", 28.0, Mm(25.0), Mm(245.0), &font);
    cover.use_text("Technical Whitepaper", 18.0, Mm(25.0), Mm(220.0), &font);
    cover.use_text("Version 1.0", 12.0, Mm(25.0), Mm(205.0), &font);
    cover.use_text(
        "Cryptographic Evidence Infrastructure",
        12.0,
        Mm(25.0),
        Mm(190.0),
        &font,
    );
    cover.use_text(
        format!("{}", current_date),
        10.0,
        Mm(25.0),
        Mm(170.0),
        &font,
    );

    add_footer(&cover, &font, 1);

    //
    // TABLE OF CONTENTS
    //

    let (page2, layer2) = doc.add_page(Mm(210.0), Mm(297.0), "Contents");
    let toc = doc.get_page(page2).get_layer(layer2);

    toc.use_text("Table of Contents", 20.0, Mm(25.0), Mm(260.0), &font);

    let sections = [
        "1. Introduction",
        "2. System Architecture",
        "3. Cryptographic Model",
        "4. Verification Process",
        "5. Security Model",
        "6. Enterprise Deployment",
    ];

    let mut toc_y = 230.0;
    for item in sections {
        toc.use_text(item, 12.0, Mm(35.0), Mm(toc_y), &font);
        toc_y -= 12.0;
    }

    add_footer(&toc, &font, 2);

    //
    // CONTENT PAGES (pagination + word wrap + tables)
    //

    let page_height_top: f32 = 270.0;
    let page_height_bottom: f32 = 35.0;
    let line_height: f32 = 6.0;
    let table_row_height: f32 = 7.0;

    let mut page_num = 3;
    let mut y = page_height_top;

    let (mut current_page, mut current_layer) = doc.add_page(Mm(210.0), Mm(297.0), "Whitepaper");

    let mut layer = doc.get_page(current_page).get_layer(current_layer);

    let mut table_row_index: usize = 0;
    let mut was_table_line = false;

    let ensure_space = |y: &mut f32,
                        page_num: &mut usize,
                        current_page: &mut PdfPageIndex,
                        current_layer: &mut PdfLayerIndex,
                        layer: &mut PdfLayerReference,
                        doc: &PdfDocumentReference,
                        font: &IndirectFontRef| {
        if *y < page_height_bottom {
            add_footer(layer, font, *page_num);
            *page_num += 1;
            let (new_page, new_layer) = doc.add_page(Mm(210.0), Mm(297.0), "Whitepaper");
            *current_page = new_page;
            *current_layer = new_layer;
            *layer = doc.get_page(*current_page).get_layer(*current_layer);
            *y = page_height_top;
        }
    };

    for raw_line in text.lines() {
        if let Some(cells) = parse_table_row(raw_line) {
            was_table_line = true;

            if is_separator_row(&cells) {
                continue;
            }

            ensure_space(
                &mut y,
                &mut page_num,
                &mut current_page,
                &mut current_layer,
                &mut layer,
                &doc,
                &font,
            );

            let row_font = if table_row_index == 0 {
                &font_bold
            } else {
                &font
            };

            let c0 = cells.get(0).map(|s| s.as_str()).unwrap_or("");
            let c1 = cells.get(1).map(|s| s.as_str()).unwrap_or("");
            let c2 = cells.get(2).map(|s| s.as_str()).unwrap_or("");

            layer.use_text(c0, 9.0, Mm(TABLE_COL1_X), Mm(y), row_font);
            layer.use_text(c1, 9.0, Mm(TABLE_COL2_X), Mm(y), row_font);
            layer.use_text(c2, 9.0, Mm(TABLE_COL3_X), Mm(y), row_font);

            y -= table_row_height;
            table_row_index += 1;
            continue;
        }

        if was_table_line {
            // left the table, add a little breathing room and reset
            y -= 2.0;
            table_row_index = 0;
            was_table_line = false;
        }

        let clean = raw_line.replace("#", "").replace("*", "");

        if clean.trim().is_empty() {
            continue;
        }

        for wrapped in wrap_line(clean.trim(), MAX_CHARS_PER_LINE) {
            ensure_space(
                &mut y,
                &mut page_num,
                &mut current_page,
                &mut current_layer,
                &mut layer,
                &doc,
                &font,
            );
            layer.use_text(wrapped, 10.0, Mm(25.0), Mm(y), &font);
            y -= line_height;
        }
    }

    add_footer(&layer, &font, page_num);

    //
    // SAVE
    //

    let file = File::create(Path::new(output)).expect("Cannot create PDF file");

    doc.save(&mut BufWriter::new(file))
        .expect("Cannot save PDF");
}

fn add_footer(layer: &PdfLayerReference, font: &IndirectFontRef, page: usize) {
    layer.use_text(
        format!(
            "Evident Ledger Technical Whitepaper v1.0 | Confidential Technical Documentation | Page {}",
            page
        ),
        8.0,
        Mm(25.0),
        Mm(15.0),
        font,
    );
}
