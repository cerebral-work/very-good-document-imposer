//! End-to-end tests (feature `qpdf-backend`): author source PDFs with qpdf, impose them, and
//! assert structural prepress invariants on the output — valid PDF, sheet/surface counts,
//! byte-determinism, rejection of non-conformant sources, vector (Form-XObject) placement, and the
//! M1.5 emission contract: marks in a Separation `All` colorant, bleed-pull clip, slug text
//! (SPEC §8/§13).
//!
//! Run with: `cargo test -p vgdi-engine --features qpdf-backend`.
#![cfg(feature = "qpdf-backend")]

use qpdf::{
    ObjectStreamMode, QPdf, QPdfArray, QPdfDictionary, QPdfObjectLike, QPdfScalar, QPdfStream,
};
use std::path::{Path, PathBuf};
use vgdi_types::{
    Barcode, BleedMode, ColorPolicy, CropMarks, Duplex, FillOrder, JobSpec, MarkSet, NUp,
    OutputTarget, RegistrationMarks, SaddleStitch, ScaleMode, Scheme, Sheet, Slug, SourceRef,
    StepRepeat, Symbology, WorkStyle, SCHEMA_ID,
};

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("vgdi_it_{name}"))
}

/// Author a source PDF with `pages` CMYK pages; optionally include a TrimBox.
fn write_source(path: &Path, pages: usize, with_trim: bool) {
    write_source_boxed(path, pages, with_trim, false);
}

/// Author a source PDF with optional TrimBox and BleedBox (200×200 media, CMYK fill).
fn write_source_boxed(path: &Path, pages: usize, with_trim: bool, with_bleed: bool) {
    let src = QPdf::empty();
    for _ in 0..pages {
        let cs = src.new_stream(&b"0.1 0.2 0.3 0 k 5 5 190 190 re f"[..]);
        let page = src.new_dictionary();
        page.set("/Type", src.new_name("/Page"));
        page.set("/MediaBox", src.parse_object("[ 0 0 200 200 ]").unwrap());
        if with_trim {
            page.set("/TrimBox", src.parse_object("[ 10 10 190 190 ]").unwrap());
        }
        if with_bleed {
            page.set("/BleedBox", src.parse_object("[ 5 5 195 195 ]").unwrap());
        }
        page.set("/Resources", src.new_dictionary());
        page.set("/Contents", &cs);
        src.add_page(&page, false).unwrap();
    }
    let mut w = src.writer();
    w.deterministic_id(true)
        .object_stream_mode(ObjectStreamMode::Generate);
    std::fs::write(path, w.write_to_memory().unwrap()).unwrap();
}

fn job_with(src_path: &Path, scheme: Scheme) -> JobSpec {
    JobSpec {
        schema: SCHEMA_ID.to_string(),
        sources: vec![SourceRef {
            id: "body".into(),
            path: src_path.to_path_buf(),
        }],
        scheme,
        sheet: Sheet {
            size_pt: [800.0, 800.0],
            gripper_pt: 0.0,
            margin_pt: 0.0,
            work_style: WorkStyle::Sheetwise,
            flip: None,
        },
        marks: MarkSet::default(),
        color_policy: ColorPolicy::default(),
        output: OutputTarget::default(),
    }
}

fn nup(rows: u32, cols: u32, bleed: BleedMode) -> Scheme {
    Scheme::NUp(NUp {
        rows,
        cols,
        fill: FillOrder::RowMajor,
        scale: ScaleMode::Fit,
        gutter_pt: 0.0,
        rotate_to_fit: false,
        bleed_mode: bleed,
    })
}

fn nup_job(src_path: &Path, rows: u32, cols: u32) -> JobSpec {
    job_with(src_path, nup(rows, cols, BleedMode::NoBleed))
}

fn nup_scale_none(bleed: BleedMode) -> Scheme {
    Scheme::NUp(NUp {
        rows: 1,
        cols: 1,
        fill: FillOrder::RowMajor,
        scale: ScaleMode::None,
        gutter_pt: 0.0,
        rotate_to_fit: false,
        bleed_mode: bleed,
    })
}

// ---- small qpdf readers for assertions ----

fn page_resources(page: &QPdfDictionary) -> QPdfDictionary {
    QPdfDictionary::from(page.get("/Resources").expect("page has resources"))
}

fn read_box(d: &QPdfDictionary, key: &str) -> [f64; 4] {
    let arr = QPdfArray::from(d.get(key).unwrap_or_else(|| panic!("missing {key}")));
    let n = |i: usize| QPdfScalar::from(arr.get(i).unwrap()).as_f64();
    [n(0), n(1), n(2), n(3)]
}

/// The `/BBox` of the first placed Form XObject (`/X0`) on a sheet page.
fn first_form_bbox(page: &QPdfDictionary) -> [f64; 4] {
    let xobj = QPdfDictionary::from(page_resources(page).get("/XObject").unwrap());
    let x0 = QPdfStream::from(xobj.get("/X0").expect("first form present"));
    read_box(&x0.get_dictionary(), "/BBox")
}

fn content_of(page: &QPdfDictionary) -> Vec<u8> {
    page.get_page_content_data().unwrap().as_ref().to_vec()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// --------------------------------------------------------------------------------- M0/M1 core

#[test]
fn end_to_end_nup_2x2_valid_and_deterministic() {
    let src = tmp("src5.pdf");
    write_source(&src, 5, true);
    let job = nup_job(&src, 2, 2);

    let out1 = tmp("out1.pdf");
    let out2 = tmp("out2.pdf");
    vgdi_engine::impose_to_file(&job, &out1).unwrap();
    vgdi_engine::impose_to_file(&job, &out2).unwrap();

    let b1 = std::fs::read(&out1).unwrap();
    let b2 = std::fs::read(&out2).unwrap();
    assert_eq!(b1, b2, "imposed output must be byte-deterministic");
    assert!(b1.starts_with(b"%PDF-"), "output must be a PDF");

    // 5 pages at 2x2 -> 2 sheets.
    let doc = QPdf::read(&out1).unwrap();
    assert_eq!(doc.get_num_pages().unwrap(), 2);

    // The imposed sheet must place pages as Form XObjects (vector), not rasterize.
    let sheet0 = doc.get_page(0).unwrap();
    assert!(
        page_resources(&sheet0).get("/XObject").is_some(),
        "placed pages must be XObjects"
    );
}

#[test]
fn source_without_trim_or_art_is_rejected() {
    let src = tmp("notrim.pdf");
    write_source(&src, 1, false);
    let job = nup_job(&src, 2, 2);
    let out = tmp("notrim_out.pdf");
    let err = vgdi_engine::impose_to_file(&job, &out).unwrap_err();
    assert!(
        matches!(err, vgdi_engine::EngineError::NoTrimOrArt { .. }),
        "expected NoTrimOrArt, got {err:?}"
    );
}

#[test]
fn saddle_stitch_renders_one_page_per_surface() {
    let src = tmp("saddle8.pdf");
    write_source(&src, 8, true);
    let job = job_with(
        &src,
        Scheme::SaddleStitch(SaddleStitch {
            duplex: Duplex::LongEdge,
            cover: false,
            scale: ScaleMode::Fit,
            spine_pt: 0.0,
            bleed_mode: BleedMode::NoBleed,
        }),
    );
    let out = tmp("saddle8_out.pdf");
    vgdi_engine::impose_to_file(&job, &out).unwrap();
    let doc = QPdf::read(&out).unwrap();
    // 8 pages -> 2 sheets -> 4 surfaces -> 4 imposed PDF pages, each a 2-up spread.
    assert_eq!(doc.get_num_pages().unwrap(), 4);
}

#[test]
fn step_repeat_renders_tiled_sheet() {
    let src = tmp("sr.pdf");
    write_source(&src, 1, true);
    let job = job_with(
        &src,
        Scheme::StepRepeat(StepRepeat {
            max_rows: 2,
            max_cols: 3,
            h_space_pt: 10.0,
            v_space_pt: 10.0,
            bleed_mode: BleedMode::NoBleed,
            scale: ScaleMode::None,
        }),
    );
    let out = tmp("sr_out.pdf");
    vgdi_engine::impose_to_file(&job, &out).unwrap();
    let doc = QPdf::read(&out).unwrap();
    assert_eq!(
        doc.get_num_pages().unwrap(),
        1,
        "one tiled sheet for one source page"
    );
}

// ------------------------------------------------------------------------ M1.5 emission contract

#[test]
fn marks_emitted_use_separation_all_colorant() {
    // When crop/registration marks are enabled, the output sheet must declare a Separation
    // colour space named `All` and stroke the marks in it (never rich black).
    let src = tmp("marks_src.pdf");
    write_source(&src, 1, true);
    let mut job = nup_job(&src, 1, 1);
    job.marks.crop = Some(CropMarks::default());
    job.marks.registration = Some(RegistrationMarks::default());

    let out = tmp("marks_out.pdf");
    vgdi_engine::impose_to_file(&job, &out).unwrap();
    let doc = QPdf::read(&out).unwrap();
    let page = doc.get_page(0).unwrap();

    // 1. A `[/Separation /All /DeviceCMYK <tint transform>]` colour space in resources.
    let cs = QPdfDictionary::from(
        page_resources(&page)
            .get("/ColorSpace")
            .expect("marks declare a colour space"),
    );
    let sep = QPdfArray::from(cs.get("/SepAll").expect("Separation All present"));
    assert_eq!(sep.get(0).unwrap().as_name(), "/Separation");
    assert_eq!(
        sep.get(1).unwrap().as_name(),
        "/All",
        "colorant name on the wire must be `All`, not the [Registration] UI alias"
    );
    assert_eq!(sep.get(2).unwrap().as_name(), "/DeviceCMYK");

    // 2. The content stream strokes in that colour space (CS/SCN), not a rich-black fill.
    let content = content_of(&page);
    assert!(
        contains(&content, b"/SepAll CS"),
        "marks set Separation All"
    );
    assert!(contains(&content, b"SCN"), "marks set the colorant tint");
    assert!(contains(&content, b" S\n"), "marks are stroked");
}

#[test]
fn bleed_pull_extends_visible_content_to_bleed() {
    // With bleed-pull on, the placed form is clipped to the BleedBox (content past the trim
    // survives); off, it is clipped to the TrimBox.
    let src = tmp("bleed_src.pdf");
    write_source_boxed(&src, 1, true, true); // trim [10..190], bleed [5..195]

    let on = job_with(&src, nup_scale_none(BleedMode::Bleed));
    let out_on = tmp("bleed_on.pdf");
    vgdi_engine::impose_to_file(&on, &out_on).unwrap();
    let doc = QPdf::read(&out_on).unwrap();
    let bbox = first_form_bbox(&doc.get_page(0).unwrap());
    assert_eq!(
        bbox,
        [5.0, 5.0, 195.0, 195.0],
        "bleed-pull clips to BleedBox"
    );

    let off = job_with(&src, nup_scale_none(BleedMode::NoBleed));
    let out_off = tmp("bleed_off.pdf");
    vgdi_engine::impose_to_file(&off, &out_off).unwrap();
    let doc = QPdf::read(&out_off).unwrap();
    let bbox = first_form_bbox(&doc.get_page(0).unwrap());
    assert_eq!(
        bbox,
        [10.0, 10.0, 190.0, 190.0],
        "no-bleed clips to TrimBox"
    );
}

#[test]
fn bleed_pull_rejects_insufficient_gutter() {
    // 2-up with bleed-pull and a gutter narrower than 2× the (scale-none) 5pt bleed must error,
    // not let neighbouring bleeds overlap (SPEC §8.7).
    let src = tmp("bleed_gutter_src.pdf");
    write_source_boxed(&src, 1, true, true);
    let job = job_with(
        &src,
        Scheme::NUp(NUp {
            rows: 1,
            cols: 2,
            fill: FillOrder::RowMajor,
            scale: ScaleMode::None,
            gutter_pt: 4.0, // < 2 × 5pt bleed
            rotate_to_fit: false,
            bleed_mode: BleedMode::Bleed,
        }),
    );
    let out = tmp("bleed_gutter_out.pdf");
    let err = vgdi_engine::impose_to_file(&job, &out).unwrap_err();
    assert!(
        matches!(
            err,
            vgdi_engine::EngineError::InsufficientBleedGutter { .. }
        ),
        "expected InsufficientBleedGutter, got {err:?}"
    );
}

#[test]
fn slug_fields_render_text_in_slug_area() {
    // The slug renders resolved info tokens (filename, sheet #, surface) as a Helvetica text run.
    let src = tmp("slugsrc.pdf");
    write_source(&src, 1, true);
    let mut job = nup_job(&src, 1, 1);
    job.marks.slug = Some(Slug::default());

    let out = tmp("slug_out.pdf");
    vgdi_engine::impose_to_file(&job, &out).unwrap();
    let doc = QPdf::read(&out).unwrap();
    let page = doc.get_page(0).unwrap();

    // A Helvetica font resource is declared with a single-byte encoding that matches the bytes we
    // emit (so the middot delimiter / non-ASCII filenames render correctly).
    let fonts = QPdfDictionary::from(page_resources(&page).get("/Font").expect("slug font"));
    let f1 = QPdfDictionary::from(fonts.get("/F1").unwrap());
    assert_eq!(f1.get("/BaseFont").unwrap().as_name(), "/Helvetica");
    assert_eq!(f1.get("/Encoding").unwrap().as_name(), "/WinAnsiEncoding");

    // The content stream draws text containing the resolved slug data.
    let content = content_of(&page);
    assert!(contains(&content, b"BT"), "text block present");
    assert!(contains(&content, b"/F1"), "slug uses the declared font");
    assert!(contains(&content, b"Tj"), "text is shown");
    assert!(
        contains(&content, b"slugsrc.pdf"),
        "slug renders the resolved filename token"
    );
    assert!(
        contains(&content, b"front"),
        "slug renders the surface token"
    );
    // The middot delimiter is emitted as its single WinAnsi byte (octal-escaped), never as the
    // raw two-byte UTF-8 sequence a StandardEncoding font would mis-render.
    assert!(
        !contains(&content, &[0xC2, 0xB7]),
        "no raw UTF-8 middot in the literal"
    );
    assert!(
        contains(&content, b"\\267"),
        "middot emitted as WinAnsi 0xB7"
    );
}

#[test]
fn job_barcode_emits_code128_bars() {
    let src = tmp("bc_src.pdf");
    write_source(&src, 1, true);
    let mut job = nup_job(&src, 1, 1);
    job.marks.job_barcode = Some(Barcode {
        symbology: Symbology::Code128,
        data: "JOB-4471".into(),
        ..Barcode::default()
    });
    let out = tmp("bc_out.pdf");
    vgdi_engine::impose_to_file(&job, &out).unwrap();
    let doc = QPdf::read(&out).unwrap();
    let content = content_of(&doc.get_page(0).unwrap());
    // Bars are filled rects in DeviceCMYK black.
    assert!(contains(&content, b" re f"), "barcode bars are filled");
    assert!(contains(&content, b"0 0 0 1 k"), "bars use device black");
}

#[test]
fn marks_output_is_byte_deterministic() {
    let src = tmp("det_src.pdf");
    write_source(&src, 2, true);
    let mut job = nup_job(&src, 1, 1);
    job.marks.crop = Some(CropMarks::default());
    job.marks.registration = Some(RegistrationMarks::default());
    job.marks.slug = Some(Slug::default());

    let a = tmp("det_a.pdf");
    let b = tmp("det_b.pdf");
    vgdi_engine::impose_to_file(&job, &a).unwrap();
    vgdi_engine::impose_to_file(&job, &b).unwrap();
    assert_eq!(
        std::fs::read(&a).unwrap(),
        std::fs::read(&b).unwrap(),
        "imposed output with marks must stay byte-identical"
    );
}
