//! qpdf-backed source reader + PDF writer (feature `qpdf-backend`).
//!
//! The page→form-XObject placement primitive is assembled on the C-API object model
//! (`copy_from_foreign` of the page's resources + the page's content stream wrapped as a Form
//! XObject), because qpdf's C++-only `getFormXObjectForPage` helper is not exposed by the C API
//! or the `qpdf` crate (ADR-0001 §2). Each placed page is wrapped in an *isolated* transparency
//! group carrying the resolved blend `/CS` (SPEC §8 invariant #4).
//!
//! Determinism (SPEC §13): deterministic `/ID`, fixed object-stream + compression modes, no
//! `/CreationDate`/`/ModDate`, and stable number formatting via [`crate::geom::fmt`].

use crate::boxes::PageBoxes;
use crate::error::{EngineError, Result};
use crate::geom::fmt;
use crate::marks::{MarkPlan, MarkPrimitive};
use crate::plan::{plan, GroupCs, ImpositionPlan, PageGeometry, SourceInfo};
use qpdf::{
    ObjectStreamMode, QPdf, QPdfArray, QPdfDictionary, QPdfObject, QPdfObjectLike, QPdfObjectType,
    QPdfScalar, QPdfStream,
};
use std::path::Path;
use vgdi_types::{JobSpec, MarkColor, Rect};

/// A source PDF loaded into memory along with its planning geometry.
pub struct LoadedSource {
    pub id: String,
    pub doc: QPdf,
    pub info: SourceInfo,
}

fn backend<E: std::fmt::Display>(e: E) -> EngineError {
    EngineError::Backend(e.to_string())
}

/// Resolve an inheritable page attribute (`/MediaBox`, `/CropBox`, `/Rotate`, `/Resources`)
/// by walking the `/Parent` chain, per the PDF inheritance rules.
fn get_inherited(page: &QPdfDictionary, key: &str) -> Option<QPdfObject> {
    fn live(o: QPdfObject) -> Option<QPdfObject> {
        if o.get_type() == QPdfObjectType::Null {
            None
        } else {
            Some(o)
        }
    }
    if let Some(o) = page.get(key) {
        if let Some(o) = live(o) {
            return Some(o);
        }
    }
    let mut parent = page.get("/Parent");
    while let Some(p) = parent {
        if p.get_type() != QPdfObjectType::Dictionary {
            break;
        }
        let pd = QPdfDictionary::from(p);
        if let Some(o) = pd.get(key) {
            if let Some(o) = live(o) {
                return Some(o);
            }
        }
        parent = pd.get("/Parent");
    }
    None
}

/// Read a 4-number rectangle box from a page dict (with inheritance for the relevant keys).
fn read_box(page: &QPdfDictionary, key: &str) -> Option<Rect> {
    let obj = get_inherited(page, key)?;
    if obj.get_type() != QPdfObjectType::Array {
        return None;
    }
    let arr = qpdf::QPdfArray::from(obj);
    if arr.len() < 4 {
        return None;
    }
    let n = |i: usize| -> f64 { QPdfScalar::from(arr.get(i).unwrap()).as_f64() };
    Some(Rect::new(n(0), n(1), n(2), n(3)))
}

fn read_page_geometry(page: &QPdfDictionary) -> Option<PageGeometry> {
    let media = read_box(page, "/MediaBox")?;
    let rotate = get_inherited(page, "/Rotate")
        .map(|o| QPdfScalar::from(o).as_i32())
        .unwrap_or(0);
    Some(PageGeometry {
        boxes: PageBoxes {
            media,
            crop: read_box(page, "/CropBox"),
            trim: read_box(page, "/TrimBox"),
            art: read_box(page, "/ArtBox"),
            bleed: read_box(page, "/BleedBox"),
        },
        rotate: rotate.rem_euclid(360),
        group_cs: detect_group_cs(page),
    })
}

/// Detect the blend color space the source page declares via its `/Group /CS`, so the engine's
/// isolated wrapper group uses a matching space instead of forcing DeviceCMYK (SPEC §8 #4). Falls
/// back to DeviceCMYK when the page declares no group space (the prepress device default).
/// NB: ICC profiles are mapped to a device family by component count — preserving the exact ICC
/// profile on the wrapper is deferred prepress work.
fn detect_group_cs(page: &QPdfDictionary) -> GroupCs {
    let Some(group) = page.get("/Group") else {
        return GroupCs::DeviceCmyk;
    };
    if group.get_type() != QPdfObjectType::Dictionary {
        return GroupCs::DeviceCmyk;
    }
    let Some(cs) = QPdfDictionary::from(group).get("/CS") else {
        return GroupCs::DeviceCmyk;
    };
    match cs.get_type() {
        QPdfObjectType::Name => match cs.as_name().trim_start_matches('/') {
            "DeviceRGB" | "CalRGB" | "RGB" => GroupCs::DeviceRgb,
            "DeviceGray" | "CalGray" | "G" => GroupCs::DeviceGray,
            _ => GroupCs::DeviceCmyk,
        },
        QPdfObjectType::Array => {
            // ICCBased: [ /ICCBased <stream> ]; the stream's /N is the component count.
            let n = QPdfArray::from(cs)
                .get(1)
                .filter(|o| o.get_type() == QPdfObjectType::Stream)
                .and_then(|o| QPdfStream::from(o).get_dictionary().get("/N"))
                .map(|o| QPdfScalar::from(o).as_i32());
            match n {
                Some(1) => GroupCs::DeviceGray,
                Some(3) => GroupCs::DeviceRgb,
                _ => GroupCs::DeviceCmyk,
            }
        }
        _ => GroupCs::DeviceCmyk,
    }
}

/// Read a source PDF and gather its per-page geometry.
pub fn read_source(id: &str, path: &Path) -> Result<LoadedSource> {
    let doc = QPdf::read(path).map_err(backend)?;
    let pages = doc.get_pages().map_err(backend)?;
    let mut geoms = Vec::with_capacity(pages.len());
    for (i, page) in pages.iter().enumerate() {
        let g = read_page_geometry(page).ok_or_else(|| {
            EngineError::Backend(format!("source `{id}` page {i}: missing/invalid MediaBox"))
        })?;
        geoms.push(g);
    }
    Ok(LoadedSource {
        id: id.to_string(),
        doc,
        info: SourceInfo {
            id: id.to_string(),
            pages: geoms,
        },
    })
}

fn group_object(dst: &QPdf, cs: GroupCs) -> Result<QPdfObject> {
    dst.parse_object(&format!(
        "<< /Type /Group /S /Transparency /I true /CS /{} >>",
        cs.pdf_name()
    ))
    .map_err(backend)
}

fn rect_object(dst: &QPdf, r: &Rect) -> Result<QPdfObject> {
    dst.parse_object(&format!(
        "[ {} {} {} {} ]",
        fmt(r.llx),
        fmt(r.lly),
        fmt(r.urx),
        fmt(r.ury)
    ))
    .map_err(backend)
}

// ---------------------------------------------------------------------------------- mark emission

/// Resource needs for a surface's marks: which colour-space / font resources must be declared.
#[derive(Default)]
struct MarkResources {
    /// A `[/Separation /All /DeviceCMYK …]` colorant is referenced (cut/reg marks; SPEC §13).
    sep_all: bool,
    /// A Helvetica font is referenced (slug text).
    font: bool,
    /// Distinct named spot colorants referenced; emitted as `/Spot{i}` resources.
    spots: Vec<String>,
}

/// Scan a [`MarkPlan`] for the colour spaces and fonts its operators will reference.
fn scan_resources(plan: &MarkPlan) -> MarkResources {
    let mut res = MarkResources::default();
    let mut note = |c: &MarkColor| match c {
        MarkColor::RegistrationAll => res.sep_all = true,
        MarkColor::Spot(name) => {
            if !res.spots.iter().any(|s| s == name) {
                res.spots.push(name.clone());
            }
        }
        MarkColor::Process { .. } => {}
    };
    for p in &plan.primitives {
        match p {
            MarkPrimitive::Line { color, .. }
            | MarkPrimitive::Rect { color, .. }
            | MarkPrimitive::Circle { color, .. }
            | MarkPrimitive::FillRect { color, .. } => note(color),
        }
    }
    res.font = !plan.texts.is_empty();
    for t in &plan.texts {
        note(&t.color);
    }
    res
}

/// The content-stream colour operator for a mark colour (`stroke` selects upper/lower-case ops).
fn color_op(color: &MarkColor, stroke: bool, res: &MarkResources) -> String {
    match color {
        MarkColor::RegistrationAll => {
            if stroke {
                "/SepAll CS 1 SCN".into()
            } else {
                "/SepAll cs 1 scn".into()
            }
        }
        MarkColor::Process { c, m, y, k } => {
            let comps = format!("{} {} {} {}", fmt(*c), fmt(*m), fmt(*y), fmt(*k));
            format!("{comps} {}", if stroke { "K" } else { "k" })
        }
        MarkColor::Spot(name) => {
            let i = res.spots.iter().position(|s| s == name).unwrap_or(0);
            if stroke {
                format!("/Spot{i} CS 1 SCN")
            } else {
                format!("/Spot{i} cs 1 scn")
            }
        }
    }
}

/// Approximate a circle as four cubic Béziers (kappa = 0.5523), counter-clockwise from `(cx+r, cy)`.
fn circle_path(cx: f64, cy: f64, r: f64) -> String {
    let k = 0.552_284_749_830_793_6 * r;
    let p = |x: f64, y: f64| format!("{} {}", fmt(x), fmt(y));
    format!(
        "{} m {} {} {} c {} {} {} c {} {} {} c {} {} {} c",
        p(cx + r, cy),
        p(cx + r, cy + k),
        p(cx + k, cy + r),
        p(cx, cy + r),
        p(cx - k, cy + r),
        p(cx - r, cy + k),
        p(cx - r, cy),
        p(cx - r, cy - k),
        p(cx - k, cy - r),
        p(cx, cy - r),
        p(cx + k, cy - r),
        p(cx + r, cy - k),
        p(cx + r, cy),
    )
}

/// Transcode a Unicode string to single-byte WinAnsi (CP1252) code units — the encoding declared on
/// the slug font. ASCII and Latin-1 (0xA0–0xFF) map directly; a few common punctuation code points
/// map to their CP1252 slots; anything else becomes `?`. This keeps the bytes we emit in agreement
/// with the font's `/Encoding`, so each byte renders as exactly one intended glyph.
fn to_winansi(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| {
            let u = c as u32;
            if (0x20..=0x7E).contains(&u) || (0xA0..=0xFF).contains(&u) {
                u as u8
            } else {
                match u {
                    0x20AC => 0x80, // €
                    0x2026 => 0x85, // …
                    0x2018 => 0x91, // ‘
                    0x2019 => 0x92, // ’
                    0x201C => 0x93, // “
                    0x201D => 0x94, // ”
                    0x2022 => 0x95, // •
                    0x2013 => 0x96, // –
                    0x2014 => 0x97, // —
                    _ => b'?',
                }
            }
        })
        .collect()
}

/// Escape WinAnsi bytes for a PDF literal `( … )`: backslash + parens escaped, printable ASCII
/// verbatim, everything else (incl. 0x80–0xFF) as octal `\ooo` so the literal stays clean ASCII.
fn escape_pdf_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            b'\\' => out.push_str("\\\\"),
            b'(' => out.push_str("\\("),
            b')' => out.push_str("\\)"),
            0x20..=0x7E => out.push(b as char),
            _ => out.push_str(&format!("\\{b:03o}")),
        }
    }
    out
}

/// Emit a surface's mark plan as content-stream operators (already in sheet space). Wrapped in
/// `q … Q` so the cells' graphics state is untouched.
fn emit_mark_ops(plan: &MarkPlan, res: &MarkResources) -> String {
    if plan.is_empty() {
        return String::new();
    }
    let mut s = String::from("q\n");
    for p in &plan.primitives {
        match p {
            MarkPrimitive::Line {
                from,
                to,
                weight,
                color,
                dash,
            } => {
                s.push_str(&color_op(color, true, res));
                s.push_str(&format!("\n{} w\n", fmt(*weight)));
                if *dash > 0.0 {
                    s.push_str(&format!("[{} {}] 0 d\n", fmt(*dash), fmt(*dash)));
                } else {
                    s.push_str("[] 0 d\n");
                }
                s.push_str(&format!(
                    "{} {} m {} {} l S\n",
                    fmt(from.0),
                    fmt(from.1),
                    fmt(to.0),
                    fmt(to.1)
                ));
            }
            MarkPrimitive::Rect {
                rect,
                weight,
                color,
            } => {
                s.push_str(&color_op(color, true, res));
                s.push_str(&format!("\n{} w\n[] 0 d\n", fmt(*weight)));
                s.push_str(&format!(
                    "{} {} {} {} re S\n",
                    fmt(rect.llx),
                    fmt(rect.lly),
                    fmt(rect.width()),
                    fmt(rect.height())
                ));
            }
            MarkPrimitive::Circle {
                center,
                radius,
                weight,
                color,
            } => {
                s.push_str(&color_op(color, true, res));
                s.push_str(&format!("\n{} w\n[] 0 d\n", fmt(*weight)));
                s.push_str(&circle_path(center.0, center.1, *radius));
                s.push_str(" S\n");
            }
            MarkPrimitive::FillRect { rect, color } => {
                s.push_str(&color_op(color, false, res));
                s.push_str(&format!(
                    "\n{} {} {} {} re f\n",
                    fmt(rect.llx),
                    fmt(rect.lly),
                    fmt(rect.width()),
                    fmt(rect.height())
                ));
            }
        }
    }
    for t in &plan.texts {
        s.push_str(&color_op(&t.color, false, res));
        s.push_str("\nBT\n");
        s.push_str(&format!("/F1 {} Tf\n", fmt(t.size)));
        s.push_str(&format!("{} {} Td\n", fmt(t.x), fmt(t.y)));
        s.push_str(&format!(
            "({}) Tj\nET\n",
            escape_pdf_bytes(&to_winansi(&t.text))
        ));
    }
    s.push_str("Q\n");
    s
}

/// `[/Separation /All /DeviceCMYK <tint→100% of every process colorant>]` (SPEC §13). The tint
/// transform is a Type-2 function mapping tint t → (t, t, t, t).
fn separation_all(dst: &QPdf) -> Result<QPdfObject> {
    dst.parse_object(
        "[ /Separation /All /DeviceCMYK \
         << /FunctionType 2 /Domain [ 0 1 ] /C0 [ 0 0 0 0 ] /C1 [ 1 1 1 1 ] /N 1 >> ]",
    )
    .map_err(backend)
}

/// A named-spot Separation colour space. The DeviceCMYK alternate (100% K at full tint) is a
/// placeholder — preserving the source's true spot tint transform is deferred prepress work.
fn separation_spot(dst: &QPdf, name: &str) -> Result<QPdfObject> {
    dst.parse_object(&format!(
        "[ /Separation /{} /DeviceCMYK \
         << /FunctionType 2 /Domain [ 0 1 ] /C0 [ 0 0 0 0 ] /C1 [ 0 0 0 1 ] /N 1 >> ]",
        pdf_name_escape(name)
    ))
    .map_err(backend)
}

/// Encode an arbitrary string as the body of a PDF name (regular chars pass through; others as
/// `#xx`), so spot colorant names with spaces/specials stay valid name tokens.
fn pdf_name_escape(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
            out.push(b as char);
        } else {
            out.push_str(&format!("#{b:02X}"));
        }
    }
    out
}

/// Render a plan to PDF bytes using the loaded sources.
pub fn render(job: &JobSpec, plan: &ImpositionPlan, sources: &[LoadedSource]) -> Result<Vec<u8>> {
    let dst = QPdf::empty();
    // The form `/Matrix` is identity for every placed cell — parse once, share by reference.
    let identity_matrix = dst
        .parse_object("[ 1 0 0 1 0 0 ]")
        .map_err(backend)?
        .into_indirect();

    for sheet in &plan.sheets {
        for surface in &sheet.surfaces {
            let xobjects = dst.new_dictionary();
            let mut ops = String::new();

            for (i, cell) in surface.cells.iter().enumerate() {
                let src = sources
                    .iter()
                    .find(|s| s.id == cell.source_id)
                    .ok_or_else(|| EngineError::UnknownSource(cell.source_id.clone()))?;
                let page = src.doc.get_page(cell.source_page as u32).ok_or_else(|| {
                    EngineError::Backend(format!("page {} vanished", cell.source_page))
                })?;

                // Page content (decoded, concatenated) becomes the form's content stream.
                let content = page.get_page_content_data().map_err(backend)?;
                let form = dst.new_stream(content.as_ref());
                let fd = form.get_dictionary();
                fd.set("/Type", dst.new_name("/XObject"));
                fd.set("/Subtype", dst.new_name("/Form"));
                fd.set("/FormType", dst.new_integer(1));
                fd.set("/BBox", rect_object(&dst, &cell.bbox)?);
                fd.set("/Matrix", &identity_matrix);
                // Copy the page's resources verbatim into the destination (preserves CMYK/spot/ICC).
                // `copy_from_foreign` requires an INDIRECT object; promote direct resources first.
                match get_inherited(&page, "/Resources") {
                    Some(res) => {
                        let res = if res.is_indirect() {
                            res
                        } else {
                            res.into_indirect()
                        };
                        fd.set("/Resources", dst.copy_from_foreign(&res));
                    }
                    None => {
                        fd.set("/Resources", dst.new_dictionary());
                    }
                }
                // Isolated wrapper group with the resolved blend space (engine-synthesized).
                fd.set("/Group", group_object(&dst, cell.group_cs)?);

                let name = format!("/X{i}");
                xobjects.set(&name, &form);
                ops.push_str(&format!("q {} cm {} Do Q\n", cell.ctm.to_pdf(), name));
            }

            // Marks/furniture: stroke the planned primitives + slug text in sheet space, after the
            // placed pages, and declare the colour-space / font resources they reference.
            let mark_res = scan_resources(&surface.marks);
            ops.push_str(&emit_mark_ops(&surface.marks, &mark_res));

            let content_stream = dst.new_stream(ops.as_bytes());
            let resources = dst.new_dictionary();
            resources.set("/XObject", &xobjects);
            if mark_res.sep_all || !mark_res.spots.is_empty() {
                let cs = dst.new_dictionary();
                if mark_res.sep_all {
                    cs.set("/SepAll", separation_all(&dst)?);
                }
                for (i, name) in mark_res.spots.iter().enumerate() {
                    cs.set(&format!("/Spot{i}"), separation_spot(&dst, name)?);
                }
                resources.set("/ColorSpace", &cs);
            }
            if mark_res.font {
                let fonts = dst.new_dictionary();
                fonts.set(
                    "/F1",
                    dst.parse_object(
                        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica \
                         /Encoding /WinAnsiEncoding >>",
                    )
                    .map_err(backend)?,
                );
                resources.set("/Font", &fonts);
            }

            let page = dst.new_dictionary();
            page.set("/Type", dst.new_name("/Page"));
            page.set(
                "/MediaBox",
                rect_object(&dst, &Rect::new(0.0, 0.0, sheet.width, sheet.height))?,
            );
            page.set("/Resources", &resources);
            page.set("/Contents", &content_stream);
            dst.add_page(&page, false).map_err(backend)?;
        }
    }

    let mut w = dst.writer();
    w.deterministic_id(true)
        .object_stream_mode(ObjectStreamMode::Generate)
        .compress_streams(true)
        .preserve_unreferenced_objects(false)
        .force_pdf_version(job.output.pdf_version.as_deref().unwrap_or("1.6"));
    w.write_to_memory().map_err(backend)
}

/// Full pipeline: read sources referenced by `job`, plan, render, write to `out`.
pub fn impose_to_file(job: &JobSpec, out: &Path) -> Result<()> {
    if job.sources.is_empty() {
        return Err(EngineError::NoSources);
    }
    let loaded: Vec<LoadedSource> = job
        .sources
        .iter()
        .map(|s| read_source(&s.id, &s.path))
        .collect::<Result<_>>()?;
    let infos: Vec<SourceInfo> = loaded.iter().map(|l| l.info.clone()).collect();
    let plan = plan(job, &infos)?;
    let bytes = render(job, &plan, &loaded)?;
    std::fs::write(out, &bytes).map_err(|e| EngineError::Io(e.to_string()))?;
    Ok(())
}
