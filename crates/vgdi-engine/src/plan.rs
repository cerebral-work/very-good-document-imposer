//! The pure planner: `JobSpec` + source page geometry -> `ImpositionPlan`.
//!
//! Deterministic and PDF-backend-independent. Dispatches per scheme and emits *surfaces of placed
//! cells*; the backend renders one PDF page per surface, so every scheme reaches PDF through the
//! same placement path (M1 design).

use crate::boxes::PageBoxes;
use crate::error::{EngineError, Result};
use crate::geom::{place_best, place_manual, transform_rect_bounds, Matrix};
use crate::imposition::{perfect_bound_order, saddle_order};
use crate::marks::{FoldLine, MarkContext, MarkPlan, PlacedCell, SurfaceMarkInput};
use vgdi_types::{
    BleedMode, Duplex, FillOrder, JobSpec, Manual, NUp, PerfectBound, Rect, ScaleMode, Scheme,
    StepRepeat, SurfaceSide, SCHEMA_ID,
};

/// Geometry of one source page needed for planning.
#[derive(Clone, Copy, Debug)]
pub struct PageGeometry {
    pub boxes: PageBoxes,
    /// PDF `/Rotate` (validated to a multiple of 90 by the planner).
    pub rotate: i32,
    /// Blend color space the source page declares (else DeviceCMYK), for the isolated wrapper.
    pub group_cs: GroupCs,
}

/// All pages of one input source.
#[derive(Clone, Debug)]
pub struct SourceInfo {
    pub id: String,
    pub pages: Vec<PageGeometry>,
}

/// Blend color space for a placed page's isolated transparency-group wrapper (SPEC §8 #4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupCs {
    DeviceCmyk,
    DeviceRgb,
    DeviceGray,
}

impl GroupCs {
    pub fn pdf_name(&self) -> &'static str {
        match self {
            GroupCs::DeviceCmyk => "DeviceCMYK",
            GroupCs::DeviceRgb => "DeviceRGB",
            GroupCs::DeviceGray => "DeviceGray",
        }
    }
}

/// One placed source page on a surface.
#[derive(Clone, Debug)]
pub struct Cell {
    pub source_id: String,
    pub source_page: usize,
    /// Painting CTM: `q <ctm> cm /Xn Do Q`.
    pub ctm: Matrix,
    /// Form `/BBox` clip in page space (trim, or bleed band for bleed-pull).
    pub bbox: Rect,
    /// Source trim box (page space) — what the page is anchored on; for mark framing.
    pub trim: Rect,
    /// Source bleed box (page space) — falls back to trim when absent; for mark framing.
    pub bleed: Rect,
    pub group_cs: GroupCs,
}

/// One side of a sheet holding placed cells.
#[derive(Clone, Debug)]
pub struct Surface {
    pub side: SurfaceSide,
    pub cells: Vec<Cell>,
    /// Printer marks/furniture for this surface, in sheet space (filled by [`attach_marks`]).
    pub marks: MarkPlan,
}

/// One output sheet (one or two surfaces).
#[derive(Clone, Debug)]
pub struct PlannedSheet {
    pub width: f64,
    pub height: f64,
    pub surfaces: Vec<Surface>,
}

/// The deterministic computed imposition result.
#[derive(Clone, Debug)]
pub struct ImpositionPlan {
    pub sheets: Vec<PlannedSheet>,
}

impl ImpositionPlan {
    /// Total emitted PDF pages = total surfaces.
    pub fn surface_count(&self) -> usize {
        self.sheets.iter().map(|s| s.surfaces.len()).sum()
    }
    pub fn cell_count(&self) -> usize {
        self.sheets
            .iter()
            .flat_map(|s| &s.surfaces)
            .map(|s| s.cells.len())
            .sum()
    }
}

/// Plan an imposition.
pub fn plan(job: &JobSpec, sources: &[SourceInfo]) -> Result<ImpositionPlan> {
    if job.schema != SCHEMA_ID {
        return Err(EngineError::SchemaMismatch {
            expected: SCHEMA_ID.to_string(),
            got: job.schema.clone(),
        });
    }
    if job.sources.is_empty() || sources.is_empty() {
        return Err(EngineError::NoSources);
    }

    let mut plan = match &job.scheme {
        Scheme::NUp(n) => plan_nup(job, sources, n),
        Scheme::StepRepeat(sr) => plan_step_repeat(job, sources, sr),
        Scheme::SaddleStitch(ss) => {
            let order = saddle_order(primary(sources)?.pages.len());
            plan_booklet(
                job,
                sources,
                ss.scale,
                ss.duplex,
                ss.spine_pt,
                ss.bleed_mode,
                &order,
            )
        }
        Scheme::PerfectBound(pb) => plan_perfect(job, sources, pb),
        Scheme::Manual(m) => plan_manual(job, sources, m),
    }?;
    attach_marks(&mut plan, job);
    Ok(plan)
}

/// Per-signature sheet span for collation numbering (perfect bind): each gathered signature is
/// `signature_pages` pages → `ceil(signature_pages / 4)` duplex sheets.
fn sheets_per_signature(job: &JobSpec) -> Option<usize> {
    match &job.scheme {
        Scheme::PerfectBound(pb) => {
            let sig = (pb.signature_pages as usize).max(4);
            Some(sig.div_ceil(4))
        }
        Scheme::SaddleStitch(_) => None, // one signature
        _ => None,
    }
}

/// Compute and attach the per-surface [`MarkPlan`] once the full plan (and its sheet numbering) is
/// known. Marks are sheet-level furniture, so this runs after scheme dispatch.
fn attach_marks(plan: &mut ImpositionPlan, job: &JobSpec) {
    if marks_all_off(&job.marks) {
        return;
    }
    let file_name = job
        .sources
        .first()
        .and_then(|s| s.path.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let booklet = matches!(
        job.scheme,
        Scheme::SaddleStitch(_) | Scheme::PerfectBound(_)
    );
    // Collation back-step marks order *gathered* signatures, so they apply to perfect binding only —
    // a saddle-stitch booklet is a single nested signature with nothing to collate.
    let per_sig = sheets_per_signature(job);

    for (sheet_idx, sheet) in plan.sheets.iter_mut().enumerate() {
        let sheet_rect = Rect::new(0.0, 0.0, sheet.width, sheet.height);
        let signature = per_sig.map(|n| sheet_idx / n.max(1) + 1);
        for surface in &mut sheet.surfaces {
            let placed: Vec<PlacedCell> = surface
                .cells
                .iter()
                .map(|c| PlacedCell {
                    trim: transform_rect_bounds(&c.ctm, c.trim),
                    bleed: transform_rect_bounds(&c.ctm, c.bleed),
                })
                .collect();
            let fold_lines = if booklet {
                spine_folds(&placed, sheet_rect)
            } else {
                Vec::new()
            };
            let input = SurfaceMarkInput {
                cells: &placed,
                fold_lines: &fold_lines,
                sheet: sheet_rect,
                gripper: job.sheet.gripper_pt,
                folded: booklet,
                marks: &job.marks,
                ctx: MarkContext {
                    file_name: &file_name,
                    sheet_number: sheet_idx + 1,
                    surface: surface.side,
                    signature,
                },
            };
            surface.marks = crate::marks::plan_surface_marks(&input);
        }
    }
}

/// The booklet spine fold. A 1×2 spread always splits the sheet width evenly around a centred spine
/// gutter, so the spine sits at the sheet's horizontal centre — robust even on a half-blank spread
/// (a single placed page from a blank pad), which still needs its spine fold + collation anchor.
/// Spans the placed pages' combined bleed height. Empty only when nothing is placed.
fn spine_folds(placed: &[PlacedCell], sheet: Rect) -> Vec<FoldLine> {
    if placed.is_empty() {
        return Vec::new();
    }
    let x = sheet.center().0;
    let y0 = placed
        .iter()
        .map(|c| c.bleed.lly)
        .fold(f64::INFINITY, f64::min);
    let y1 = placed
        .iter()
        .map(|c| c.bleed.ury)
        .fold(f64::NEG_INFINITY, f64::max);
    vec![FoldLine::Vertical { x, y0, y1 }]
}

/// True when no mark family is enabled (lets the planner skip the mark pass entirely).
fn marks_all_off(m: &vgdi_types::MarkSet) -> bool {
    m.crop.is_none()
        && m.center.is_none()
        && !m.trim_outline
        && m.registration.is_none()
        && m.fold.is_none()
        && m.collation.is_none()
        && m.color_bar.is_none()
        && m.slug.is_none()
        && m.job_barcode.is_none()
        && matches!(m.bleed, vgdi_types::BleedTreatment::None)
}

fn primary(sources: &[SourceInfo]) -> Result<&SourceInfo> {
    sources.first().ok_or(EngineError::NoSources)
}

/// Validate a source page and return its (trim, effective-bleed, rotate, group-cs).
fn validate_page(src: &SourceInfo, page: usize) -> Result<(Rect, Rect, i32, GroupCs)> {
    let geom = src
        .pages
        .get(page)
        .ok_or_else(|| EngineError::EmptySource { id: src.id.clone() })?;
    let trim = geom
        .boxes
        .effective_trim()
        .ok_or_else(|| EngineError::NoTrimOrArt {
            id: src.id.clone(),
            page,
        })?;
    if !geom.boxes.containment_ok() {
        return Err(EngineError::ContainmentViolation {
            id: src.id.clone(),
            page,
        });
    }
    if geom.rotate % 90 != 0 {
        return Err(EngineError::InvalidRotate {
            id: src.id.clone(),
            page,
            rotate: geom.rotate,
        });
    }
    let bleed = geom.boxes.effective_bleed().unwrap_or(trim);
    Ok((trim, bleed, geom.rotate, geom.group_cs))
}

/// Rectangle for grid cell (row, col); row 0 at top. The grid fills the imageable area = the sheet
/// inset by `margin` on all four edges, with the gripper additionally reserved on the bottom edge,
/// so sheet-edge marks have room (SPEC §8.6).
#[allow(clippy::too_many_arguments)]
fn grid_cell_rect(
    sheet_w: f64,
    sheet_h: f64,
    gripper: f64,
    margin: f64,
    rows: u32,
    cols: u32,
    h_gap: f64,
    v_gap: f64,
    r: u32,
    c: u32,
) -> Result<Rect> {
    let usable_w = sheet_w - 2.0 * margin;
    let usable_h = sheet_h - gripper - 2.0 * margin;
    let cell_w = (usable_w - h_gap * (cols as f64 - 1.0)) / cols as f64;
    let cell_h = (usable_h - v_gap * (rows as f64 - 1.0)) / rows as f64;
    if cell_w <= 0.0 {
        return Err(EngineError::SheetTooSmall { axis: "x" });
    }
    if cell_h <= 0.0 {
        return Err(EngineError::SheetTooSmall { axis: "y" });
    }
    let llx = margin + c as f64 * (cell_w + h_gap);
    let from_top = r as f64;
    let lly = gripper + margin + (rows as f64 - 1.0 - from_top) * (cell_h + v_gap);
    Ok(Rect::new(llx, lly, llx + cell_w, lly + cell_h))
}

fn slot_to_rowcol(slot: usize, rows: u32, cols: u32, fill: FillOrder) -> (u32, u32) {
    let slot = slot as u32 % (rows * cols);
    match fill {
        FillOrder::RowMajor => (slot / cols, slot % cols),
        FillOrder::ColMajor => (slot % rows, slot / rows),
    }
}

#[allow(clippy::too_many_arguments)]
fn make_cell(
    src_id: &str,
    page: usize,
    ctm: Matrix,
    bbox: Rect,
    trim: Rect,
    bleed: Rect,
    group_cs: GroupCs,
) -> Cell {
    Cell {
        source_id: src_id.to_string(),
        source_page: page,
        ctm,
        bbox,
        trim,
        bleed,
        group_cs,
    }
}

// ----------------------------------------------------------------------------------- N-up

fn plan_nup(job: &JobSpec, sources: &[SourceInfo], n: &NUp) -> Result<ImpositionPlan> {
    if n.rows == 0 || n.cols == 0 {
        return Err(EngineError::EmptyGrid {
            rows: n.rows,
            cols: n.cols,
        });
    }
    let src = primary(sources)?;
    if src.pages.is_empty() {
        return Err(EngineError::EmptySource { id: src.id.clone() });
    }
    let (sw, sh) = (job.sheet.size_pt[0], job.sheet.size_pt[1]);
    let per_sheet = (n.rows * n.cols) as usize;
    let has_neighbor = n.rows > 1 || n.cols > 1;

    let mut sheets = Vec::new();
    let mut cells = Vec::new();
    for page in 0..src.pages.len() {
        let (trim, bleed, rot, cs) = validate_page(src, page)?;
        let slot = cells.len();
        let (r, c) = slot_to_rowcol(slot, n.rows, n.cols, n.fill);
        let cell = grid_cell_rect(
            sw,
            sh,
            job.sheet.gripper_pt,
            job.sheet.margin_pt,
            n.rows,
            n.cols,
            n.gutter_pt,
            n.gutter_pt,
            r,
            c,
        )?;
        let p = place_best(trim, rot, cell, n.scale, n.rotate_to_fit, false);
        // Bleed-pull: clip to the BleedBox band so content past the trim survives (SPEC §8.7).
        let clip = match n.bleed_mode {
            BleedMode::Bleed => bleed,
            BleedMode::NoBleed => trim,
        };
        if matches!(n.bleed_mode, BleedMode::Bleed) && has_neighbor {
            check_bleed_gutter(&p.ctm, trim, bleed, n.gutter_pt)?;
        }
        cells.push(make_cell(&src.id, page, p.ctm, clip, trim, bleed, cs));
        if cells.len() == per_sheet {
            sheets.push(one_surface_sheet(sw, sh, std::mem::take(&mut cells)));
        }
    }
    if !cells.is_empty() {
        sheets.push(one_surface_sheet(sw, sh, cells));
    }
    Ok(ImpositionPlan { sheets })
}

fn one_surface_sheet(w: f64, h: f64, cells: Vec<Cell>) -> PlannedSheet {
    PlannedSheet {
        width: w,
        height: h,
        surfaces: vec![Surface {
            side: SurfaceSide::Front,
            cells,
            marks: MarkPlan::default(),
        }],
    }
}

/// Reject bleed-pull when the trim gutter is narrower than two placed bleed bands (neighbouring
/// bleeds would overlap). Compares in sheet space, so cell scaling is accounted for (SPEC §8.7).
fn check_bleed_gutter(ctm: &Matrix, trim: Rect, bleed: Rect, gutter: f64) -> Result<()> {
    let pt = transform_rect_bounds(ctm, trim);
    let pb = transform_rect_bounds(ctm, bleed);
    let amt = (pt.llx - pb.llx)
        .max(pt.lly - pb.lly)
        .max(pb.urx - pt.urx)
        .max(pb.ury - pt.ury)
        .max(0.0);
    let needed = 2.0 * amt;
    if gutter + 1e-6 < needed {
        return Err(EngineError::InsufficientBleedGutter { gutter, needed });
    }
    Ok(())
}

// --------------------------------------------------------------------------- step & repeat

fn plan_step_repeat(
    job: &JobSpec,
    sources: &[SourceInfo],
    sr: &StepRepeat,
) -> Result<ImpositionPlan> {
    if sr.rows == 0 || sr.cols == 0 {
        return Err(EngineError::EmptyGrid {
            rows: sr.rows,
            cols: sr.cols,
        });
    }
    let src = primary(sources)?;
    if src.pages.is_empty() {
        return Err(EngineError::EmptySource { id: src.id.clone() });
    }
    let (sw, sh) = (job.sheet.size_pt[0], job.sheet.size_pt[1]);

    // One sheet per source page, fully tiled with that page (gang one design per sheet).
    let mut sheets = Vec::new();
    for page in 0..src.pages.len() {
        let (trim, bleed, rot, cs) = validate_page(src, page)?;
        let clip = match sr.bleed_mode {
            BleedMode::Bleed => bleed,
            BleedMode::NoBleed => trim,
        };
        let mut cells = Vec::new();
        for r in 0..sr.rows {
            for c in 0..sr.cols {
                let cell = grid_cell_rect(
                    sw,
                    sh,
                    job.sheet.gripper_pt,
                    job.sheet.margin_pt,
                    sr.rows,
                    sr.cols,
                    sr.h_space_pt,
                    sr.v_space_pt,
                    r,
                    c,
                )?;
                let p = place_best(trim, rot, cell, sr.scale, false, false);
                cells.push(make_cell(&src.id, page, p.ctm, clip, trim, bleed, cs));
            }
        }
        sheets.push(one_surface_sheet(sw, sh, cells));
    }
    Ok(ImpositionPlan { sheets })
}

// -------------------------------------------------------------------- saddle / perfect bind

fn plan_perfect(
    job: &JobSpec,
    sources: &[SourceInfo],
    pb: &PerfectBound,
) -> Result<ImpositionPlan> {
    let order = perfect_bound_order(primary(sources)?.pages.len(), pb.signature_pages as usize);
    plan_booklet(
        job,
        sources,
        pb.scale,
        pb.duplex,
        pb.spine_pt,
        pb.bleed_mode,
        &order,
    )
}

/// Form `/BBox` (page space) for a booklet page with spine-safe bleed: the three outer edges pull
/// to the BleedBox, the spine edge (right for the left page `col == 0`, left for the right page)
/// stays at the TrimBox. Computed in sheet space then mapped back through the inverse CTM, so page
/// `/Rotate` and the duplex flip are handled correctly.
fn spine_safe_clip(ctm: &Matrix, trim: Rect, bleed: Rect, col: usize) -> Rect {
    let pt = transform_rect_bounds(ctm, trim);
    let pb = transform_rect_bounds(ctm, bleed);
    let sheet_clip = if col == 0 {
        Rect::new(pb.llx, pb.lly, pt.urx, pb.ury) // left page: spine on the right
    } else {
        Rect::new(pt.llx, pb.lly, pb.urx, pb.ury) // right page: spine on the left
    };
    transform_rect_bounds(&ctm.inverse(), sheet_clip)
}

#[allow(clippy::too_many_arguments)]
fn plan_booklet(
    job: &JobSpec,
    sources: &[SourceInfo],
    scale: ScaleMode,
    duplex: Duplex,
    spine: f64,
    bleed_mode: BleedMode,
    order: &[(SurfaceSide, [usize; 2])],
) -> Result<ImpositionPlan> {
    let src = primary(sources)?;
    if src.pages.is_empty() {
        return Err(EngineError::EmptySource { id: src.id.clone() });
    }
    let (sw, sh) = (job.sheet.size_pt[0], job.sheet.size_pt[1]);
    let page_count = src.pages.len();
    let short_edge = matches!(duplex, Duplex::ShortEdge);

    // Each surface is a 1x2 spread (left, right) with a spine gutter between.
    let mut surfaces: Vec<Surface> = Vec::with_capacity(order.len());
    for (side, [left, right]) in order {
        let flip = *side == SurfaceSide::Back && short_edge;
        let mut cells = Vec::new();
        for (col, &pnum) in [*left, *right].iter().enumerate() {
            if pnum == 0 || pnum > page_count {
                continue; // blank pad
            }
            let page = pnum - 1;
            let (trim, bleed, rot, cs) = validate_page(src, page)?;
            let cell = grid_cell_rect(
                sw,
                sh,
                job.sheet.gripper_pt,
                job.sheet.margin_pt,
                1,
                2,
                spine,
                0.0,
                0,
                col as u32,
            )?;
            let p = place_best(trim, rot, cell, scale, false, flip);
            // Spine-safe bleed-pull: bleed the three outer (cut) edges, but keep the spine edge —
            // the fold between the two pages of the spread — clipped to trim (SPEC §8.7).
            let clip = match bleed_mode {
                BleedMode::NoBleed => p.bbox,
                BleedMode::Bleed => spine_safe_clip(&p.ctm, trim, bleed, col),
            };
            cells.push(make_cell(&src.id, page, p.ctm, clip, trim, bleed, cs));
        }
        surfaces.push(Surface {
            side: *side,
            cells,
            marks: MarkPlan::default(),
        });
    }

    // Pair front+back surfaces into sheets (print order is already F,B,F,B,…).
    let sheets = surfaces
        .chunks(2)
        .map(|pair| PlannedSheet {
            width: sw,
            height: sh,
            surfaces: pair.to_vec(),
        })
        .collect();
    Ok(ImpositionPlan { sheets })
}

// --------------------------------------------------------------------------------- manual

fn plan_manual(job: &JobSpec, sources: &[SourceInfo], m: &Manual) -> Result<ImpositionPlan> {
    let (sw, sh) = (job.sheet.size_pt[0], job.sheet.size_pt[1]);
    let mut surfaces = Vec::new();
    for ms in &m.surfaces {
        let mut cells = Vec::new();
        for pl in &ms.placements {
            let src = match &pl.source {
                Some(id) => sources
                    .iter()
                    .find(|s| &s.id == id)
                    .ok_or_else(|| EngineError::UnknownSource(id.clone()))?,
                None => primary(sources)?,
            };
            let (trim, bleed, rot, cs) = validate_page(src, pl.page)?;
            if pl.rotate % 90 != 0 {
                return Err(EngineError::InvalidRotate {
                    id: src.id.clone(),
                    page: pl.page,
                    rotate: pl.rotate,
                });
            }
            let p = place_manual(trim, pl.x_pt, pl.y_pt, pl.scale, pl.rotate + rot, pl.mirror);
            cells.push(make_cell(&src.id, pl.page, p.ctm, p.bbox, trim, bleed, cs));
        }
        surfaces.push(Surface {
            side: ms.side,
            cells,
            marks: MarkPlan::default(),
        });
    }
    // Each manual surface is its own sheet/page.
    let sheets = surfaces
        .into_iter()
        .map(|s| PlannedSheet {
            width: sw,
            height: sh,
            surfaces: vec![s],
        })
        .collect();
    Ok(ImpositionPlan { sheets })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vgdi_types::*;

    fn geom(trim: Option<Rect>, rotate: i32) -> PageGeometry {
        PageGeometry {
            boxes: PageBoxes {
                media: Rect::new(0.0, 0.0, 200.0, 200.0),
                crop: None,
                trim,
                art: None,
                bleed: Some(Rect::new(5.0, 5.0, 195.0, 195.0)),
            },
            rotate,
            group_cs: GroupCs::DeviceCmyk,
        }
    }

    fn src(n: usize) -> Vec<SourceInfo> {
        let pages = (0..n)
            .map(|_| geom(Some(Rect::new(10.0, 10.0, 190.0, 190.0)), 0))
            .collect();
        vec![SourceInfo {
            id: "body".into(),
            pages,
        }]
    }

    fn job(scheme: Scheme) -> JobSpec {
        JobSpec {
            schema: SCHEMA_ID.to_string(),
            sources: vec![SourceRef {
                id: "body".into(),
                path: "b.pdf".into(),
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

    fn nup(rows: u32, cols: u32) -> Scheme {
        Scheme::NUp(NUp {
            rows,
            cols,
            fill: FillOrder::RowMajor,
            scale: ScaleMode::Fit,
            gutter_pt: 0.0,
            rotate_to_fit: false,
            bleed_mode: BleedMode::NoBleed,
        })
    }

    fn saddle() -> Scheme {
        Scheme::SaddleStitch(SaddleStitch {
            duplex: Duplex::LongEdge,
            cover: false,
            scale: ScaleMode::Fit,
            spine_pt: 0.0,
            bleed_mode: BleedMode::NoBleed,
        })
    }

    #[test]
    fn nup_5_pages_2x2_two_sheets_one_surface_each() {
        let p = plan(&job(nup(2, 2)), &src(5)).unwrap();
        assert_eq!(p.sheets.len(), 2);
        assert_eq!(p.sheets[0].surfaces.len(), 1);
        assert_eq!(p.sheets[0].surfaces[0].cells.len(), 4);
        assert_eq!(p.sheets[1].surfaces[0].cells.len(), 1);
        assert_eq!(p.surface_count(), 2);
        assert_eq!(p.cell_count(), 5);
    }

    #[test]
    fn sheet_margin_insets_the_imposition_grid() {
        // A 50pt imageable margin insets the grid to [50..750] on an 800×800 sheet, so a 1-up Fit
        // page fills that inset area (leaving the band for sheet-edge marks) rather than the sheet.
        let mut j = job(nup(1, 1));
        j.sheet.margin_pt = 50.0;
        let p = plan(&j, &src(1)).unwrap();
        let cell = &p.sheets[0].surfaces[0].cells[0];
        let placed = crate::geom::transform_rect_bounds(&cell.ctm, cell.trim);
        assert_eq!(placed, Rect::new(50.0, 50.0, 750.0, 750.0));
    }

    #[test]
    fn nup_rejects_page_without_trim() {
        let mut s = src(0);
        s[0].pages.push(geom(None, 0));
        assert!(matches!(
            plan(&job(nup(2, 2)), &s).unwrap_err(),
            EngineError::NoTrimOrArt { .. }
        ));
    }

    #[test]
    fn step_repeat_tiles_each_page_on_its_own_sheet() {
        let sr = Scheme::StepRepeat(StepRepeat {
            rows: 3,
            cols: 4,
            h_space_pt: 10.0,
            v_space_pt: 10.0,
            bleed_mode: BleedMode::Bleed,
            scale: ScaleMode::Fit,
        });
        let p = plan(&job(sr), &src(2)).unwrap();
        assert_eq!(p.sheets.len(), 2, "one sheet per source page");
        assert_eq!(p.sheets[0].surfaces[0].cells.len(), 12, "3x4 = 12 repeats");
    }

    #[test]
    fn step_repeat_bleed_mode_uses_bleed_box_as_clip() {
        let mk = |mode| {
            Scheme::StepRepeat(StepRepeat {
                rows: 1,
                cols: 1,
                h_space_pt: 0.0,
                v_space_pt: 0.0,
                bleed_mode: mode,
                scale: ScaleMode::None,
            })
        };
        let bleed = plan(&job(mk(BleedMode::Bleed)), &src(1)).unwrap();
        let nobleed = plan(&job(mk(BleedMode::NoBleed)), &src(1)).unwrap();
        let bclip = bleed.sheets[0].surfaces[0].cells[0].bbox;
        let nclip = nobleed.sheets[0].surfaces[0].cells[0].bbox;
        assert_eq!(
            bclip,
            Rect::new(5.0, 5.0, 195.0, 195.0),
            "bleed clip = BleedBox"
        );
        assert_eq!(
            nclip,
            Rect::new(10.0, 10.0, 190.0, 190.0),
            "no-bleed clip = TrimBox"
        );
    }

    #[test]
    fn saddle_8_pages_two_sheets_front_back() {
        let p = plan(&job(saddle()), &src(8)).unwrap();
        assert_eq!(p.sheets.len(), 2, "8 pages -> 2 sheets");
        assert_eq!(p.surface_count(), 4, "front+back each");
        // sheet 0: front spread = pages 8 & 1 (source indices 7 & 0).
        let front = &p.sheets[0].surfaces[0];
        assert_eq!(front.side, SurfaceSide::Front);
        let mut idxs: Vec<usize> = front.cells.iter().map(|c| c.source_page).collect();
        idxs.sort();
        assert_eq!(idxs, vec![0, 7]);
    }

    #[test]
    fn saddle_6_pages_drops_blank_pads() {
        // 6 pages padded to 8; pages 7,8 are blank -> fewer cells than slots.
        let p = plan(&job(saddle()), &src(6)).unwrap();
        assert_eq!(p.cell_count(), 6, "only real pages get cells");
        assert_eq!(p.surface_count(), 4);
    }

    fn perfect(sig: u32) -> Scheme {
        Scheme::PerfectBound(PerfectBound {
            signature_pages: sig,
            duplex: Duplex::LongEdge,
            scale: ScaleMode::Fit,
            spine_pt: 0.0,
            bleed_mode: BleedMode::NoBleed,
        })
    }

    #[test]
    fn perfect_bound_collation_tabs_anchor_on_sheet_centre_spine() {
        // 6 pages, signature 8 → one padded signature with single-cell (blank-pad) spreads. Every
        // collation back-step tab, including on single-cell spreads, anchors on the spine = centre.
        let mut j = job(perfect(8));
        j.marks.collation = Some(CollationMarks::default());
        let p = plan(&j, &src(6)).unwrap();
        let mut tabs = 0;
        for sheet in &p.sheets {
            for surf in &sheet.surfaces {
                for prim in &surf.marks.primitives {
                    if let crate::marks::MarkPrimitive::FillRect { rect, .. } = prim {
                        let cx = (rect.llx + rect.urx) / 2.0;
                        assert!(
                            (cx - 400.0).abs() < 1e-6,
                            "collation tab off-spine: cx={cx} (expected sheet centre 400)"
                        );
                        tabs += 1;
                    }
                }
            }
        }
        assert_eq!(tabs, 4, "one back-step tab per surface");
    }

    #[test]
    fn saddle_stitch_emits_no_collation_marks() {
        // Saddle stitch is a single nested signature — nothing to collate, so no back-step tabs even
        // when the mark is enabled.
        let mut j = job(saddle());
        j.marks.collation = Some(CollationMarks::default());
        let p = plan(&j, &src(6)).unwrap();
        let any_tab = p
            .sheets
            .iter()
            .flat_map(|s| &s.surfaces)
            .flat_map(|s| &s.marks.primitives)
            .any(|p| matches!(p, crate::marks::MarkPrimitive::FillRect { .. }));
        assert!(!any_tab, "saddle stitch must not emit collation tabs");
    }

    #[test]
    fn booklet_crop_marks_frame_the_spread_not_the_spine() {
        // The spine (sheet centre x=400) is a fold: no crop tick may originate near it — crop marks
        // frame the whole 2-up leaf's outer perimeter only, even on half-blank (single-cell) spreads
        // (6 pages pad to 8 → two single-cell spreads).
        let mut j = job(saddle());
        j.marks.crop = Some(CropMarks::default());
        let p = plan(&j, &src(6)).unwrap();
        for sheet in &p.sheets {
            for surf in &sheet.surfaces {
                for prim in &surf.marks.primitives {
                    if let crate::marks::MarkPrimitive::Line { from, to, .. } = prim {
                        assert!(
                            (from.0 - 400.0).abs() > 30.0 && (to.0 - 400.0).abs() > 30.0,
                            "crop tick at the spine fold: {from:?}->{to:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn saddle_bleed_clips_spine_edge_to_trim_outer_edges_to_bleed() {
        // src() trim=[10,10,190,190], bleed=[5,5,195,195]. With spine-safe bleed each page bleeds on
        // its three outer edges but keeps the spine (fold) edge at trim.
        let j = job(Scheme::SaddleStitch(SaddleStitch {
            duplex: Duplex::LongEdge,
            cover: false,
            scale: ScaleMode::None,
            spine_pt: 40.0,
            bleed_mode: BleedMode::Bleed,
        }));
        let p = plan(&j, &src(4)).unwrap(); // [4,1] front + [2,3] back → all 2-cell spreads
        let surf = &p.sheets[0].surfaces[0];
        assert_eq!(surf.cells.len(), 2);
        let mut cells: Vec<&Cell> = surf.cells.iter().collect();
        cells.sort_by(|a, b| {
            transform_rect_bounds(&a.ctm, a.trim)
                .llx
                .partial_cmp(&transform_rect_bounds(&b.ctm, b.trim).llx)
                .unwrap()
        });
        let (left, right) = (cells[0].bbox, cells[1].bbox);
        // Left page: spine on the right → right edge at trim (190), the other three at bleed.
        assert!((left.urx - 190.0).abs() < 1e-6, "left spine edge at trim");
        assert!((left.llx - 5.0).abs() < 1e-6, "left outer edge at bleed");
        assert!((left.lly - 5.0).abs() < 1e-6 && (left.ury - 195.0).abs() < 1e-6);
        // Right page: spine on the left → left edge at trim (10), the other three at bleed.
        assert!((right.llx - 10.0).abs() < 1e-6, "right spine edge at trim");
        assert!(
            (right.urx - 195.0).abs() < 1e-6,
            "right outer edge at bleed"
        );
        assert!((right.lly - 5.0).abs() < 1e-6 && (right.ury - 195.0).abs() < 1e-6);
    }

    #[test]
    fn perfect_bound_32_pages_8_per_sig() {
        let pb = Scheme::PerfectBound(PerfectBound {
            signature_pages: 8,
            duplex: Duplex::LongEdge,
            scale: ScaleMode::Fit,
            spine_pt: 0.0,
            bleed_mode: BleedMode::NoBleed,
        });
        let p = plan(&job(pb), &src(32)).unwrap();
        assert_eq!(p.surface_count(), 16, "4 signatures x 4 surfaces");
        assert_eq!(p.cell_count(), 32);
    }

    #[test]
    fn manual_places_explicit_cells() {
        let m = Scheme::Manual(Manual {
            surfaces: vec![ManualSurface {
                side: SurfaceSide::Front,
                placements: vec![
                    ManualPlacement {
                        source: None,
                        page: 0,
                        x_pt: 0.0,
                        y_pt: 0.0,
                        scale: 1.0,
                        rotate: 0,
                        mirror: false,
                    },
                    ManualPlacement {
                        source: None,
                        page: 0,
                        x_pt: 300.0,
                        y_pt: 0.0,
                        scale: 0.5,
                        rotate: 90,
                        mirror: true,
                    },
                ],
            }],
        });
        let p = plan(&job(m), &src(1)).unwrap();
        assert_eq!(p.sheets.len(), 1);
        assert_eq!(p.cell_count(), 2);
    }

    #[test]
    fn invalid_rotate_is_rejected_not_panicked() {
        // A parseable-but-malformed /Rotate (not a multiple of 90) must be a clean error,
        // never a panic in rotate_cw.
        let pages = vec![geom(Some(Rect::new(10.0, 10.0, 190.0, 190.0)), 45)];
        let sources = vec![SourceInfo {
            id: "body".into(),
            pages,
        }];
        assert!(matches!(
            plan(&job(nup(1, 1)), &sources).unwrap_err(),
            EngineError::InvalidRotate { rotate: 45, .. }
        ));
    }

    #[test]
    fn cell_carries_declared_group_cs() {
        // A page that declares an RGB blend space must keep it (not be forced to CMYK).
        let mut g = geom(Some(Rect::new(10.0, 10.0, 190.0, 190.0)), 0);
        g.group_cs = GroupCs::DeviceRgb;
        let sources = vec![SourceInfo {
            id: "body".into(),
            pages: vec![g],
        }];
        let p = plan(&job(nup(1, 1)), &sources).unwrap();
        assert_eq!(
            p.sheets[0].surfaces[0].cells[0].group_cs,
            GroupCs::DeviceRgb
        );
    }
}
