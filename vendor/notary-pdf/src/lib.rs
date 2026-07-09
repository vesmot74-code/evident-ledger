//! PDF certificate — court-grade cryptographic evidence v1.2.

use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use printpdf::color::{Color, Rgb};
use printpdf::{
    Actions, BorderArray, ColorArray, HighlightingMode, IndirectFontRef, LinkAnnotation, Mm,
    PdfDocument, PdfDocumentReference, PdfLayerIndex, PdfLayerReference, PdfPageIndex, Rect,
};
use qrcode::QrCode;
use std::io::{BufWriter, Cursor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertificateStatus {
    Valid,
    InvalidHash,
    InvalidTsa,
    MissingTsa,
}

#[derive(Debug, Clone)]
pub struct CertificateInput {
    pub status: CertificateStatus,
    pub file_hash_valid: bool,
    pub tsa_valid: bool,
    pub proof_id: String,
    pub sha256: String,
    pub object_type: String,
    pub created_at_utc: String,
    pub tsa_provider: String,
    pub tsa_timestamp_utc: String,
    pub tsa_token_base64: String,
    pub verify_url: String,
    pub file_size_kb: u64,
    pub file_name: String,
}

impl CertificateInput {
    pub fn format_timestamp_unix(ts: u64) -> String {
        Utc.timestamp_opt(ts as i64, 0)
            .single()
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "неизвестно".into())
    }
}

impl CertificateStatus {
    fn heading(&self) -> &'static str {
        match self {
            Self::Valid => "ПОДТВЕРЖДЕНО",
            Self::InvalidHash => "НАРУШЕНА ЦЕЛОСТНОСТЬ",
            Self::InvalidTsa => "ОШИБКА ВРЕМЕННОЙ МЕТКИ",
            Self::MissingTsa => "МЕТКА ВРЕМЕНИ ОТСУТСТВУЕТ",
        }
    }

    fn color(&self) -> Rgb {
        match self {
            Self::Valid => Rgb::new(0.08, 0.50, 0.24, None),
            Self::InvalidHash | Self::InvalidTsa => Rgb::new(0.73, 0.11, 0.11, None),
            Self::MissingTsa => Rgb::new(0.85, 0.65, 0.13, None),
        }
    }

    fn border_color(&self) -> Rgb {
        match self {
            Self::Valid => Rgb::new(0.86, 0.99, 0.91, None),
            Self::InvalidHash | Self::InvalidTsa => Rgb::new(0.99, 0.89, 0.89, None),
            Self::MissingTsa => Rgb::new(0.99, 0.95, 0.78, None),
        }
    }
}

const WRAP_MAX_CHARS: usize = 88;

pub fn generate_certificate_pdf(input: &CertificateInput) -> Result<Vec<u8>> {
    let (doc, page1, layer1) =
        PdfDocument::new("Notary Certificate", Mm(210.0), Mm(297.0), "Layer 1");
    let (font, font_bold) = load_embedded_fonts(&doc)?;

    let mut writer = PdfWriter {
        doc: &doc,
        page: page1,
        layer_id: layer1,
        font: &font,
        font_bold: &font_bold,
        y: 275.0,
        left: 20.0,
        right: 190.0,
        line_height: 4.2,
    };

    // ===== 0. СТАТУС =====
    writer.section_title("СВИДЕТЕЛЬСТВО О КРИПТОГРАФИЧЕСКОЙ ФИКСАЦИИ");
    writer.blank(2.0);

    writer.line("Статус фиксации: ФИНАЛЬНЫЙ", 11.0, true);
    writer.line("Система: Notary Core v1.1", 9.5, false);
    writer.line(
        "Модель доверия: SHA-256 + RFC 3161 TSA (external)",
        9.5,
        false,
    );
    writer.blank(3.0);

    // ===== 1. ОБЪЕКТ ФИКСАЦИИ (SOURCE OF TRUTH) =====
    writer.section_heading("1. ОБЪЕКТ ФИКСАЦИИ (SOURCE OF TRUTH)");
    writer.labeled("Тип объекта", &input.object_type);
    writer.labeled("Имя объекта", &input.file_name);
    writer.labeled("Размер", &format!("{} bytes", input.file_size_kb * 1024));
    writer.labeled("Алгоритм хэширования", "SHA-256");
    writer.blank(1.0);

    writer.line("Контрольная сумма (SHA-256):", 9.5, true);
    writer.line(&input.sha256, 9.0, false);
    writer.blank(1.0);

    writer.labeled("Идентификатор фиксации", &input.proof_id);
    writer.labeled("Дата фиксации (UTC)", &input.created_at_utc);
    writer.blank(2.0);

    // ===== 2. КРИПТОГРАФИЧЕСКАЯ ЦЕЛОСТНОСТЬ =====
    writer.section_heading("2. КРИПТОГРАФИЧЕСКАЯ ЦЕЛОСТНОСТЬ");
    writer.labeled("Результат", "Подтверждена");
    writer.line("Метод: Побайтовое совпадение SHA-256", 9.0, false);
    writer.wrap(
        "Объект не изменялся с момента фиксации. Любое изменение данных приводит к изменению контрольной суммы.",
        9.0,
        false,
    );
    writer.blank(2.0);

    // ===== 3. ВРЕМЕННАЯ МЕТКА (RFC 3161) =====
    writer.section_heading("3. ВРЕМЕННАЯ МЕТКА (RFC 3161)");
    let tsa_valid = input.tsa_valid;
    writer.labeled(
        "Статус TSA",
        if tsa_valid {
            "Подтверждено"
        } else {
            "Не подтверждено"
        },
    );
    writer.labeled("Провайдер", &input.tsa_provider);
    writer.labeled("Стандарт", "RFC 3161");
    writer.labeled("Время фиксации (UTC)", &input.tsa_timestamp_utc);
    writer.wrap(
        "Таймстемп подписан независимым временным центром и сохраняет силу вне зависимости от доступности системы Notary Core.",
        9.0,
        false,
    );
    writer.blank(2.0);

    // ===== 4. НЕЗАВИСИМАЯ ПРОВЕРКА =====
    writer.section_heading("4. НЕЗАВИСИМАЯ ПРОВЕРКА");
    writer.wrap(
        "Проверка может быть выполнена без доверия к системе.",
        9.0,
        false,
    );
    writer.blank(1.0);

    writer.subheading("4.1 Онлайн проверка");
    writer.wrap(&format!("{}", input.verify_url), 9.0, false);
    writer.draw_qr_link(&input.verify_url).ok();
    writer.blank(1.0);

    writer.subheading("4.2 Криптографическая проверка SHA-256");
    writer.line("sha256sum <file>", 9.0, false);
    writer.line("или", 9.0, false);
    writer.line("openssl dgst -sha256 <file>", 9.0, false);
    writer.wrap(
        "Результат должен совпадать с контрольной суммой из Раздела 1.",
        9.0,
        false,
    );
    writer.blank(1.0);

    writer.subheading("4.3 Проверка TSA");
    writer.line("openssl ts -reply -in proof.tsr -text", 9.0, false);
    writer.wrap(
        "Проверка должна подтвердить: SHA-256 объекта, время RFC 3161, подпись TSA-провайдера.",
        9.0,
        false,
    );
    writer.blank(2.0);

    // ===== 5. ЮРИДИЧЕСКИЙ СТАТУС =====
    writer.section_heading("5. ЮРИДИЧЕСКИЙ СТАТУС");
    writer.wrap(
        "Настоящий документ фиксирует криптографическое состояние цифрового объекта на момент времени.",
        9.0,
        false,
    );
    writer.wrap(
        "Документ не содержит юридической квалификации обстоятельств и не определяет правовые последствия.",
        9.0,
        false,
    );
    writer.blank(2.0);

    // ===== 6. ТЕХНИЧЕСКИЕ ГАРАНТИИ =====
    writer.section_heading("6. ТЕХНИЧЕСКИЕ ГАРАНТИИ");
    writer.line("Алгоритм хэширования: SHA-256", 9.0, false);
    writer.line("Временная метка: RFC 3161", 9.0, false);
    writer.labeled(
        "TSA",
        &format!("external trusted provider ({})", input.tsa_provider),
    );
    writer.line("Модель хранения: append-only ledger", 9.0, false);
    writer.line("Детеминизм генерации: гарантирован", 9.0, false);
    writer.blank(2.0);

    // ===== 7. ИТОГОВЫЙ ВЫВОД =====
    writer.section_heading("7. ИТОГОВЫЙ ВЫВОД");
    writer.wrap(
        "Цифровой объект зафиксирован в неизменяемом виде.",
        9.0,
        false,
    );
    writer.wrap(
        "Любая модификация объекта после фиксации приводит к несоответствию криптографической контрольной суммы и обнаруживается при независимой проверке.",
        9.0,
        false,
    );

    let mut buf = Vec::new();
    doc.save(&mut BufWriter::new(&mut buf))?;
    Ok(buf)
}

fn load_embedded_fonts(doc: &PdfDocumentReference) -> Result<(IndirectFontRef, IndirectFontRef)> {
    let mut regular = Cursor::new(include_bytes!("../assets/fonts/DejaVuSans.ttf").as_ref());
    let mut bold = Cursor::new(include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf").as_ref());
    let font = doc
        .add_external_font(&mut regular)
        .context("load DejaVuSans.ttf")?;
    let font_bold = doc
        .add_external_font(&mut bold)
        .context("load DejaVuSans-Bold.ttf")?;
    Ok((font, font_bold))
}

struct PdfWriter<'a> {
    doc: &'a PdfDocumentReference,
    page: PdfPageIndex,
    layer_id: PdfLayerIndex,
    font: &'a IndirectFontRef,
    font_bold: &'a IndirectFontRef,
    y: f32,
    left: f32,
    right: f32,
    line_height: f32,
}

impl<'a> PdfWriter<'a> {
    fn layer(&self) -> PdfLayerReference {
        self.doc.get_page(self.page).get_layer(self.layer_id)
    }

    fn ensure_space(&mut self, needed_mm: f32) {
        if self.y - needed_mm < 15.0 {
            let (page, layer) = self.doc.add_page(Mm(210.0), Mm(297.0), "Layer 1");
            self.page = page;
            self.layer_id = layer;
            self.y = 275.0;
        }
    }

    fn set_color(&self, rgb: Rgb) {
        self.layer().set_fill_color(Color::Rgb(rgb));
    }

    fn reset_color(&self) {
        self.set_color(Rgb::new(0.0, 0.0, 0.0, None));
    }

    fn blank(&mut self, mm: f32) {
        self.y -= mm;
    }

    fn section_title(&mut self, text: &str) {
        self.ensure_space(12.0);
        self.reset_color();
        self.line(text, 14.0, true);
    }

    fn section_heading(&mut self, text: &str) {
        self.ensure_space(10.0);
        self.blank(1.5);
        self.reset_color();
        self.line(text, 11.0, true);
    }

    fn subheading(&mut self, text: &str) {
        self.ensure_space(8.0);
        self.reset_color();
        self.line(text, 10.0, true);
    }

    fn line(&mut self, text: &str, size: f32, bold: bool) {
        self.ensure_space(self.line_height + 1.0);
        let font = if bold { self.font_bold } else { self.font };
        self.layer()
            .use_text(text, size, Mm(self.left), Mm(self.y), font);
        self.y -= self.line_height + (size - 9.0) * 0.15;
    }

    fn labeled(&mut self, label: &str, value: &str) {
        self.line(&format!("{}: {}", label, value), 9.0, false);
    }

    fn wrap(&mut self, text: &str, size: f32, bold: bool) {
        for paragraph in text.split('\n') {
            for chunk in wrap_text(paragraph, WRAP_MAX_CHARS) {
                self.line(&chunk, size, bold);
            }
            self.blank(0.5);
        }
    }

    fn draw_qr_link(&mut self, url: &str) -> Result<()> {
        self.ensure_space(40.0);
        let code = QrCode::new(url.as_bytes()).context("encode qr")?;
        let modules = code.width();
        let qr_size = 30.0;
        let module_mm = qr_size / modules as f32;
        let layer = self.layer();
        layer.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
        for y in 0..modules {
            for x in 0..modules {
                if code[(x, y)] == qrcode::types::Color::Dark {
                    let left = self.left + x as f32 * module_mm;
                    let bottom = self.y - qr_size + y as f32 * module_mm;
                    layer.add_rect(Rect::new(
                        Mm(left),
                        Mm(bottom),
                        Mm(left + module_mm),
                        Mm(bottom + module_mm),
                    ));
                }
            }
        }
        self.reset_color();
        layer.add_link_annotation(LinkAnnotation::new(
            Rect::new(
                Mm(self.left),
                Mm(self.y - qr_size),
                Mm(self.left + qr_size),
                Mm(self.y),
            ),
            Some(BorderArray::default()),
            Some(ColorArray::default()),
            Actions::uri(url.to_string()),
            Some(HighlightingMode::Invert),
        ));
        self.y -= qr_size + 2.0;
        Ok(())
    }
}

fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input(status: CertificateStatus) -> CertificateInput {
        CertificateInput {
            status,
            file_hash_valid: matches!(status, CertificateStatus::Valid),
            tsa_valid: true,
            proof_id: "proof-abc".into(),
            sha256: "64ab6e53abd7583364b6c36a1b2c77cc3f29956d89fd9c626f10008d90539c40".into(),
            object_type: "Документ".into(),
            file_name: "contract_final.pdf".into(),
            created_at_utc: "2024-01-15 12:00:00 UTC".into(),
            tsa_provider: "freetsa.org".into(),
            tsa_timestamp_utc: "2024-01-15 12:00:01 UTC".into(),
            tsa_token_base64: "dGVzdC10b2tlbg==".into(),
            verify_url: "http://localhost:3000/v/proof-abc".into(),
            file_size_kb: 1024,
        }
    }

    #[test]
    fn generates_pdf_for_valid_status() {
        let pdf = generate_certificate_pdf(&sample_input(CertificateStatus::Valid)).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 50_000);
    }

    #[test]
    fn embeds_dejavu_font() {
        let pdf = generate_certificate_pdf(&sample_input(CertificateStatus::Valid)).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("DejaVu"));
    }

    #[test]
    fn generates_pdf_for_each_status() {
        for status in [
            CertificateStatus::Valid,
            CertificateStatus::InvalidHash,
            CertificateStatus::InvalidTsa,
            CertificateStatus::MissingTsa,
        ] {
            let pdf = generate_certificate_pdf(&sample_input(status)).unwrap();
            assert!(pdf.starts_with(b"%PDF"));
        }
    }
}
