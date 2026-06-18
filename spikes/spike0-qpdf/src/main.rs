//! Spike-0 — the feasibility gate that must build green on macOS, Windows (MSVC), and Linux
//! before any scheme code (SPEC §16, ADR-0001 follow-ups).
//!
//! It proves, end to end and self-contained (no fixtures):
//!   1. the vendored qpdf static-links and runs;
//!   2. a source page can be placed as a *transformed Form XObject* (the imposition primitive),
//!      assembled on the C-API object model with `copy_from_foreign` (no C++-only helper);
//!   3. the placed page is wrapped in an isolated transparency group with a blend `/CS`;
//!   4. the output is byte-deterministic (built twice → identical bytes).
//!
//! Diagnostic output only; this is a throwaway dev tool, not shipped product.

use qpdf::{ObjectStreamMode, QPdf, QPdfObjectLike};

type R<T> = Result<T, Box<dyn std::error::Error>>;

/// Build a one-page source PDF with a DeviceCMYK fill and a non-trivial TrimBox.
fn build_source() -> R<QPdf> {
    let src = QPdf::empty();
    // CMYK content — its survival through placement is the whole point.
    let content = b"0.1 0.2 0.3 0 k\n10 10 180 180 re f\n0 0 0 1 K\n2 w\n10 10 180 180 re S\n";
    let cs = src.new_stream(&content[..]);
    let page = src.new_dictionary();
    page.set("/Type", src.new_name("/Page"));
    page.set("/MediaBox", src.parse_object("[ 0 0 200 200 ]")?);
    page.set("/TrimBox", src.parse_object("[ 10 10 190 190 ]")?);
    page.set("/Resources", src.new_dictionary());
    page.set("/Contents", &cs);
    src.add_page(&page, false)?;
    Ok(src)
}

/// Impose the source page 2×2 onto a larger sheet, anchoring on its TrimBox (180×180).
fn build_imposed() -> R<QPdf> {
    let src = build_source()?;
    let dst = QPdf::empty();

    let spage = src.get_page(0).ok_or("source has no page 0")?;
    let content = spage.get_page_content_data()?;

    // Wrap the page content as a Form XObject in the destination.
    let form = dst.new_stream(content.as_ref());
    let fd = form.get_dictionary();
    fd.set("/Type", dst.new_name("/XObject"));
    fd.set("/Subtype", dst.new_name("/Form"));
    fd.set("/FormType", dst.new_integer(1));
    fd.set("/BBox", dst.parse_object("[ 10 10 190 190 ]")?); // = TrimBox
    fd.set("/Matrix", dst.parse_object("[ 1 0 0 1 0 0 ]")?);
    fd.set(
        "/Group",
        dst.parse_object("<< /Type /Group /S /Transparency /I true /CS /DeviceCMYK >>")?,
    );
    let res = spage.get("/Resources").ok_or("page has no /Resources")?;
    // copy_from_foreign requires an indirect object; promote direct resources first.
    let res = if res.is_indirect() {
        res
    } else {
        res.into_indirect()
    };
    fd.set("/Resources", dst.copy_from_foreign(&res));

    // 2×2 placement: anchor TrimBox lower-left (10,10) to each cell origin, scale 1:1.
    let cells = [(0.0, 0.0), (180.0, 0.0), (0.0, 180.0), (180.0, 180.0)];
    let xobjects = dst.new_dictionary();
    xobjects.set("/X0", &form);
    let mut ops = String::new();
    for (cx, cy) in cells {
        ops.push_str(&format!(
            "q 1 0 0 1 {} {} cm /X0 Do Q\n",
            cx - 10.0,
            cy - 10.0
        ));
    }
    let content_stream = dst.new_stream(ops.as_bytes());

    let resources = dst.new_dictionary();
    resources.set("/XObject", &xobjects);
    let page = dst.new_dictionary();
    page.set("/Type", dst.new_name("/Page"));
    page.set("/MediaBox", dst.parse_object("[ 0 0 360 360 ]")?);
    page.set("/Resources", &resources);
    page.set("/Contents", &content_stream);
    dst.add_page(&page, false)?;
    Ok(dst)
}

fn write_deterministic(doc: &QPdf) -> R<Vec<u8>> {
    let mut w = doc.writer();
    w.deterministic_id(true)
        .object_stream_mode(ObjectStreamMode::Generate)
        .compress_streams(true)
        .force_pdf_version("1.6");
    Ok(w.write_to_memory()?)
}

fn main() -> R<()> {
    println!("qpdf library version: {}", QPdf::library_version());

    let bytes_a = write_deterministic(&build_imposed()?)?;
    let bytes_b = write_deterministic(&build_imposed()?)?;
    let deterministic = bytes_a == bytes_b;

    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "spike0-out.pdf".to_string());
    std::fs::write(&out, &bytes_a)?;

    // Re-read to confirm the output is a valid, single-page PDF.
    let check = QPdf::read(&out)?;
    let pages = check.get_num_pages()?;

    println!("wrote: {out} ({} bytes)", bytes_a.len());
    println!("pages: {pages}");
    println!("deterministic: {deterministic}");

    if !deterministic {
        return Err("output is not byte-deterministic".into());
    }
    if pages != 1 {
        return Err("expected exactly 1 imposed page".into());
    }
    Ok(())
}
