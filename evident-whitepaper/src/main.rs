use chrono::Local;
use printpdf::*;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;

fn main() {
    let input = "docs/whitepaper/Evident_Ledger_Technical_Whitepaper_v1.0.md";
    let output = "docs/whitepaper/Evident_Ledger_Technical_Whitepaper_v1.0.pdf";

    println!("Evident Whitepaper Generator");
    println!("Input: {}", input);

    let content = fs::read_to_string(input)
        .expect("Cannot read whitepaper markdown file");

    generate_pdf(&content, output);

    println!("PDF generated:");
    println!("{}", output);
}

fn generate_pdf(text: &str, output: &str) {
    let (doc, page1, layer1) =
        PdfDocument::new(
            "Evident Ledger Technical Whitepaper",
            Mm(210.0),
            Mm(297.0),
            "Layer 1",
        );

    let font = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .expect("Cannot load font");

    let current_date = Local::now()
        .format("%Y-%m-%d")
        .to_string();

    let layer = doc
        .get_page(page1)
        .get_layer(layer1);

    layer.use_text(
        "Evident Ledger",
        24.0,
        Mm(25.0),
        Mm(260.0),
        &font,
    );

    layer.use_text(
        "Technical Whitepaper v1.0",
        16.0,
        Mm(25.0),
        Mm(245.0),
        &font,
    );

    layer.use_text(
        format!("Generated: {}", current_date),
        10.0,
        Mm(25.0),
        Mm(230.0),
        &font,
    );

    let mut y = 210.0;

    for line in text.lines() {
        if y < 20.0 {
            break;
        }

        let clean = line
            .replace("#", "")
            .replace("*", "");

        if !clean.trim().is_empty() {
            layer.use_text(
                clean,
                10.0,
                Mm(25.0),
                Mm(y),
                &font,
            );

            y -= 6.0;
        }
    }

    let file = File::create(Path::new(output))
        .expect("Cannot create PDF file");

    doc.save(
        &mut BufWriter::new(file)
    )
    .expect("Cannot save PDF");
}
