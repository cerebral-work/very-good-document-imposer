//! The pure planner: `JobSpec` + source page geometry -> `ImpositionPlan`.
//!
//! Deterministic and PDF-backend-independent. Dispatches per scheme and emits *surfaces of placed
//! cells*; the backend renders one PDF page per surface, so every scheme reaches PDF through the
//! same placement path (M1 design).

use crate::boxes::PageBoxes;
use crate::error::{EngineError, Result};
use crate::geom::{place_best, place_manual, Matrix};
use crate::imposition::{perfect_bound_order, saddle_order};
use vgdi_types::{
    BleedMode, Duplex, FillOrder, JobSpec, Manual, NUp, PerfectBound, Rect, ScaleMode, Scheme,
    StepRepeat, SurfaceSide, SCHEMA_ID,
};

/// Geometry of one source page needed for planning.
#[derive(Clone, Copy, Debug)]
pub struct PageGeometry {
    pub boxes: PageBoxes,
    /// PDF `/Rotate`, normalized to 0/90/180/270 by the reader.
    pub rotate: i32,
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
    /// Form `/BBox` clip in page space (trim, or bleed band for bleed modes).
    pub bbox: Rect,
    pub group_cs: GroupCs,
}

/// One side of a sheet holding placed cells.
#[derive(Clone, Debug)]
pub struct Surface {
    pub side: SurfaceSide,
    pub cells: Vec<Cell>,
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

    match &job.scheme {
        Scheme::NUp(n) => plan_nup(job, sources, n),
        Scheme::StepRepeat(sr) => plan_step_repeat(job, sources, sr),
        Scheme::SaddleStitch(ss) => {
            let order = saddle_order(primary(sources)?.pages.len());
            plan_booklet(job, sources, ss.scale, ss.duplex, ss.spine_pt, &order)
        }
        Scheme::PerfectBound(pb) => plan_perfect(job, sources, pb),
        Scheme::Manual(m) => plan_manual(job, sources, m),
    }
}

fn primary(sources: &[SourceInfo]) -> Result<&SourceInfo> {
    sources.first().ok_or(EngineError::NoSources)
}

/// Validate a source page and return its (trim, effective-bleed, rotate).
fn validate_page(src: &SourceInfo, page: usize) -> Result<(Rect, Rect, i32)> {
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
    let bleed = geom.boxes.effective_bleed().unwrap_or(trim);
    Ok((trim, bleed, geom.rotate))
}

/// Rectangle for grid cell (row, col); row 0 at top, gripper reserved at the bottom edge.
#[allow(clippy::too_many_arguments)]
fn grid_cell_rect(
    sheet_w: f64,
    sheet_h: f64,
    gripper: f64,
    rows: u32,
    cols: u32,
    h_gap: f64,
    v_gap: f64,
    r: u32,
    c: u32,
) -> Result<Rect> {
    let usable_h = sheet_h - gripper;
    let cell_w = (sheet_w - h_gap * (cols as f64 - 1.0)) / cols as f64;
    let cell_h = (usable_h - v_gap * (rows as f64 - 1.0)) / rows as f64;
    if cell_w <= 0.0 {
        return Err(EngineError::SheetTooSmall { axis: "x" });
    }
    if cell_h <= 0.0 {
        return Err(EngineError::SheetTooSmall { axis: "y" });
    }
    let llx = c as f64 * (cell_w + h_gap);
    let from_top = r as f64;
    let lly = gripper + (rows as f64 - 1.0 - from_top) * (cell_h + v_gap);
    Ok(Rect::new(llx, lly, llx + cell_w, lly + cell_h))
}

fn slot_to_rowcol(slot: usize, rows: u32, cols: u32, fill: FillOrder) -> (u32, u32) {
    let slot = slot as u32 % (rows * cols);
    match fill {
        FillOrder::RowMajor => (slot / cols, slot % cols),
        FillOrder::ColMajor => (slot % rows, slot / rows),
    }
}

fn cmyk_cell(src_id: &str, page: usize, ctm: Matrix, bbox: Rect) -> Cell {
    Cell {
        source_id: src_id.to_string(),
        source_page: page,
        ctm,
        bbox,
        group_cs: GroupCs::DeviceCmyk,
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

    let mut sheets = Vec::new();
    let mut cells = Vec::new();
    for page in 0..src.pages.len() {
        let (trim, _bleed, rot) = validate_page(src, page)?;
        let slot = cells.len();
        let (r, c) = slot_to_rowcol(slot, n.rows, n.cols, n.fill);
        let cell = grid_cell_rect(
            sw,
            sh,
            job.sheet.gripper_pt,
            n.rows,
            n.cols,
            n.gutter_pt,
            n.gutter_pt,
            r,
            c,
        )?;
        let p = place_best(trim, rot, cell, n.scale, n.rotate_to_fit, false);
        cells.push(cmyk_cell(&src.id, page, p.ctm, p.bbox));
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
        }],
    }
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
        let (trim, bleed, rot) = validate_page(src, page)?;
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
                    sr.rows,
                    sr.cols,
                    sr.h_space_pt,
                    sr.v_space_pt,
                    r,
                    c,
                )?;
                let p = place_best(trim, rot, cell, sr.scale, false, false);
                cells.push(cmyk_cell(&src.id, page, p.ctm, clip));
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
    plan_booklet(job, sources, pb.scale, pb.duplex, pb.spine_pt, &order)
}

fn plan_booklet(
    job: &JobSpec,
    sources: &[SourceInfo],
    scale: ScaleMode,
    duplex: Duplex,
    spine: f64,
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
            let (trim, _bleed, rot) = validate_page(src, page)?;
            let cell = grid_cell_rect(
                sw,
                sh,
                job.sheet.gripper_pt,
                1,
                2,
                spine,
                0.0,
                0,
                col as u32,
            )?;
            let p = place_best(trim, rot, cell, scale, false, flip);
            cells.push(cmyk_cell(&src.id, page, p.ctm, p.bbox));
        }
        surfaces.push(Surface { side: *side, cells });
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
            let (trim, _bleed, rot) = validate_page(src, pl.page)?;
            let p = place_manual(trim, pl.x_pt, pl.y_pt, pl.scale, pl.rotate + rot, pl.mirror);
            cells.push(cmyk_cell(&src.id, pl.page, p.ctm, p.bbox));
        }
        surfaces.push(Surface {
            side: ms.side,
            cells,
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
        })
    }

    fn saddle() -> Scheme {
        Scheme::SaddleStitch(SaddleStitch {
            duplex: Duplex::LongEdge,
            cover: false,
            scale: ScaleMode::Fit,
            spine_pt: 0.0,
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

    #[test]
    fn perfect_bound_32_pages_8_per_sig() {
        let pb = Scheme::PerfectBound(PerfectBound {
            signature_pages: 8,
            duplex: Duplex::LongEdge,
            scale: ScaleMode::Fit,
            spine_pt: 0.0,
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
}
