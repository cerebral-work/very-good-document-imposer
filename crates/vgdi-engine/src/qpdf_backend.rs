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
use crate::plan::{plan, GroupCs, ImpositionPlan, PageGeometry, SourceInfo};
use qpdf::{
    ObjectStreamMode, QPdf, QPdfArray, QPdfDictionary, QPdfObject, QPdfObjectLike, QPdfObjectType,
    QPdfScalar, QPdfStream,
};
use std::path::Path;
use vgdi_types::{JobSpec, Rect};

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

            let content_stream = dst.new_stream(ops.as_bytes());
            let resources = dst.new_dictionary();
            resources.set("/XObject", &xobjects);

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
