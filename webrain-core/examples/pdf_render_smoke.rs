//! Smoke test for the `pdfium` feature without the CLI/chat layers:
//! `cargo run --release -p webrain-core --features pdfium --example pdf_render_smoke -- <file.pdf>`

#[cfg(feature = "pdfium")]
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: pdf_render_smoke <file.pdf>");
    let t0 = std::time::Instant::now();
    let v = webrain_core::engines::pdf_render(&path, Some(&[1]), Some(72.0), None, None)
        .expect("pdf_render failed");
    println!(
        "ok in {:?}: pages={}",
        t0.elapsed(),
        v["pages"].as_array().map(|a| a.len()).unwrap_or(0)
    );
    // Optimized builds can block in pdfium teardown after the work is done.
    std::process::exit(0);
}

#[cfg(not(feature = "pdfium"))]
fn main() {
    eprintln!("pdf_render_smoke needs --features pdfium");
}
