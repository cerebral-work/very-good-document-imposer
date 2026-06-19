//! Author a license-clean source PDF fixture (DeviceCMYK content + TrimBox) for the M0 golden
//! corpus and manual CLI runs (SPEC §16 Spike-0 prerequisite). Throwaway dev tool.
//!
//! Usage: `gen-fixture <out.pdf> [page_count] [bleed]`
//!   - `bleed` (literal 3rd arg) adds a BleedBox [5 5 195 195] and fills content out to it, so
//!     bleed-pull has a visible 5pt band past the trim — otherwise the page is trim-only.

use qpdf::{ObjectStreamMode, QPdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fixture-src.pdf".to_string());
    let n: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let bleed = std::env::args().nth(3).is_some_and(|s| s == "bleed");

    let src = QPdf::empty();
    for i in 0..n {
        // Vary cyan per page so imposed cells are visually distinguishable. With `bleed`, fill the
        // full bleed area [5..195] and outline the trim, so the 5pt bleed band is visible past trim.
        let cyan = (i % 5) as f64 * 0.2;
        let content = if bleed {
            // Bleed band in a contrasting magenta so spine-safe bleed-pull is obvious: paint the full
            // bleed box, then the page tint over the trim box, then stroke the trim. The magenta frame
            // shows on the three outer (cut) edges and is clipped at trim on the spine fold.
            format!(
                "0 1 0.55 0 k\n5 5 190 190 re f\n\
                 {cyan} 0.15 0.1 0 k\n10 10 180 180 re f\n0 0 0 1 K\n2 w\n10 10 180 180 re S\n"
            )
        } else {
            format!("{cyan} 0.15 0.1 0 k\n10 10 180 180 re f\n0 0 0 1 K\n2 w\n10 10 180 180 re S\n")
        };
        let cs = src.new_stream(content.as_bytes());
        let page = src.new_dictionary();
        page.set("/Type", src.new_name("/Page"));
        page.set("/MediaBox", src.parse_object("[ 0 0 200 200 ]")?);
        page.set("/TrimBox", src.parse_object("[ 10 10 190 190 ]")?);
        if bleed {
            page.set("/BleedBox", src.parse_object("[ 5 5 195 195 ]")?);
        }
        page.set("/Resources", src.new_dictionary());
        page.set("/Contents", &cs);
        src.add_page(&page, false)?;
    }

    let mut w = src.writer();
    w.deterministic_id(true)
        .object_stream_mode(ObjectStreamMode::Generate)
        .compress_streams(true);
    std::fs::write(&out, w.write_to_memory()?)?;
    let bleed_note = if bleed { ", BleedBox 5..195" } else { "" };
    println!("wrote {out} ({n} pages, DeviceCMYK, TrimBox 10..190{bleed_note})");
    Ok(())
}
