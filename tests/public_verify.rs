//! UI coverage for public hash verification page (`static/verify.html`).
//!
//! The page is static HTML + client JS; these tests assert markup and the
//! hash-result wiring for Certificate PDF download (no browser runtime).

fn verify_html() -> &'static str {
    include_str!("../static/verify.html")
}

#[test]
fn hash_result_markup_reuses_chain_download_style() {
    let html = verify_html();
    assert!(
        html.contains(r#"id="hashPdfActions""#),
        "hash result must include certificate action container"
    );
    assert!(
        html.contains(r#"id="hashPdfLink""#),
        "hash result must include certificate download link"
    );
    assert!(
        html.contains(r#"class="btn btn-secondary" id="hashPdfLink""#),
        "hash PDF link must reuse chain download button classes"
    );
    assert!(
        html.contains(r#"id="chainPdfLink""#),
        "chain download action must remain present"
    );
}

#[test]
fn hash_success_wires_certificate_pdf_download_url() {
    let html = verify_html();
    // Positive path: evidence found + public_proof_id → certificate.pdf link.
    assert!(
        html.contains("/public/verify/${encodeURIComponent(proofIdRaw)}/certificate.pdf"),
        "successful hash verify must set href to /public/verify/{{public_proof_id}}/certificate.pdf"
    );
    assert!(
        html.contains("pdfActions.style.display = 'flex'"),
        "certificate actions must be shown when public_proof_id is present"
    );
}

#[test]
fn hash_not_found_keeps_certificate_download_hidden() {
    let html = verify_html();
    // Negative path scaffolding: unknown hash renders not-found copy and never
    // enables the PDF action (display starts/ stays none; early return).
    assert!(
        html.contains("noEvidenceFound"),
        "not-found state must use noEvidenceFound copy"
    );
    assert!(
        html.contains("noEvidenceDetail"),
        "not-found state must include detail message"
    );
    assert!(
        html.contains("data.exists !== true"),
        "missing evidence must take the not-found branch"
    );
    // Before any success wiring, actions are forced hidden.
    assert!(
        html.contains("pdfActions.style.display = 'none'"),
        "certificate download must stay hidden unless public_proof_id is set"
    );
    // Success-only assignment of certificate URL (not on the not-found branch).
    let success_marker = "/public/verify/${encodeURIComponent(proofIdRaw)}/certificate.pdf";
    let success_idx = html
        .find(success_marker)
        .expect("certificate URL wiring must exist");
    let not_found_idx = html
        .find("data.exists !== true")
        .expect("not-found branch must exist");
    assert!(
        not_found_idx < success_idx,
        "not-found branch must return before certificate URL is assigned"
    );
}
