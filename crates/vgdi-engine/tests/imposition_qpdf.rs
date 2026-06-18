//! End-to-end M0 test (feature `qpdf-backend`): author a source PDF with qpdf, impose it, and
//! assert the output is a valid PDF, has the expected sheet count, is byte-deterministic, and
//! that non-conformant sources are rejected (SPEC §8/§13).
//!
//! Run with: `cargo test -p vgdi-engine --features qpdf-backend`.
#![cfg(feature = "qpdf-backend")]

use qpdf::{ObjectStreamMode, QPdf};
use std::path::{Path, PathBuf};
use vgdi_types::{
    BleedMode, ColorPolicy, Duplex, FillOrder, JobSpec, MarkSet, NUp, OutputTarget, SaddleStitch,
    ScaleMode, Scheme, Sheet, SourceRef, StepRepeat, WorkStyle, SCHEMA_ID,
};

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("vgdi_it_{name}"))
}

/// Author a source PDF with `pages` CMYK pages; optionally include a TrimBox.
fn write_source(path: &Path, pages: usize, with_trim: bool) {
    let src = QPdf::empty();
    for _ in 0..pages {
        let cs = src.new_stream(&b"0.1 0.2 0.3 0 k 10 10 180 180 re f"[..]);
        let page = src.new_dictionary();
        page.set("/Type", src.new_name("/Page"));
        page.set("/MediaBox", src.parse_object("[ 0 0 200 200 ]").unwrap());
        if with_trim {
            page.set("/TrimBox", src.parse_object("[ 10 10 190 190 ]").unwrap());
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
            work_style: WorkStyle::Sheetwise,
            flip: None,
        },
        marks: MarkSet::default(),
        color_policy: ColorPolicy::default(),
        output: OutputTarget::default(),
    }
}

fn nup_job(src_path: &Path, rows: u32, cols: u32) -> JobSpec {
    job_with(
        src_path,
        Scheme::NUp(NUp {
            rows,
            cols,
            fill: FillOrder::RowMajor,
            scale: ScaleMode::Fit,
            gutter_pt: 0.0,
            rotate_to_fit: false,
        }),
    )
}

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
    let res = sheet0.get("/Resources").expect("sheet has resources");
    let res = qpdf::QPdfDictionary::from(res);
    assert!(
        res.get("/XObject").is_some(),
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
            rows: 2,
            cols: 3,
            h_space_pt: 10.0,
            v_space_pt: 10.0,
            bleed_mode: BleedMode::NoBleed,
            scale: ScaleMode::Fit,
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

// ---- Deferred emission specs (written now; un-ignore as M1 emission lands) ----

#[test]
#[ignore = "M1 pending: emit marks into the sheet content stream"]
fn marks_emitted_use_separation_all_colorant() {
    // When crop/registration marks are enabled, the output sheet must declare a Separation
    // colour space named `All` and stroke the marks in it (never rich black).
    unimplemented!("marks emission not wired into qpdf_backend yet");
}

#[test]
#[ignore = "M1 pending: bleed-pull renders content out to the bleed band"]
fn bleed_pull_extends_visible_content_to_bleed() {
    unimplemented!("bleed-pull clip rendering not wired yet");
}

#[test]
#[ignore = "M1 pending: slug text + barcode emission"]
fn slug_fields_render_text_in_slug_area() {
    unimplemented!("slug text emission not wired yet");
}
