//! Author a license-clean source PDF fixture (DeviceCMYK content + TrimBox) for the M0 golden
//! corpus and manual CLI runs (SPEC §16 Spike-0 prerequisite). Throwaway dev tool.
//!
//! Usage: `gen-fixture <out.pdf> [page_count]`

use qpdf::{ObjectStreamMode, QPdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fixture-src.pdf".to_string());
    let n: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);

    let src = QPdf::empty();
    for i in 0..n {
        // Vary cyan per page so imposed cells are visually distinguishable.
        let cyan = (i % 5) as f64 * 0.2;
        let content = format!(
            "{cyan} 0.15 0.1 0 k\n10 10 180 180 re f\n0 0 0 1 K\n2 w\n10 10 180 180 re S\n"
        );
        let cs = src.new_stream(content.as_bytes());
        let page = src.new_dictionary();
        page.set("/Type", src.new_name("/Page"));
        page.set("/MediaBox", src.parse_object("[ 0 0 200 200 ]")?);
        page.set("/TrimBox", src.parse_object("[ 10 10 190 190 ]")?);
        page.set("/Resources", src.new_dictionary());
        page.set("/Contents", &cs);
        src.add_page(&page, false)?;
    }

    let mut w = src.writer();
    w.deterministic_id(true)
        .object_stream_mode(ObjectStreamMode::Generate)
        .compress_streams(true);
    std::fs::write(&out, w.write_to_memory()?)?;
    println!("wrote {out} ({n} pages, DeviceCMYK, TrimBox 10..190)");
    Ok(())
}
