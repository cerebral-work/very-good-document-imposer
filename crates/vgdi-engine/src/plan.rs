//! The pure planner: `JobSpec` + source page geometry -> `ImpositionPlan`.
//!
//! Deterministic and PDF-backend-independent. Dispatches per scheme and emits *surfaces of placed
//! cells*; the backend renders one PDF page per surface, so every scheme reaches PDF through the
//! same placement path (M1 design).

use crate::boxes::PageBoxes;
use crate::error::{EngineError, Result};
use crate::geom::{place_best, place_manual, reflect_x, reflect_y, transform_rect_bounds, Matrix};
use crate::imposition::{perfect_bound_order, saddle_order};
use crate::marks::{
    FoldLine, GripperEdge, MarkContext, MarkPlan, PlacedCell, SurfaceLayout, SurfaceMarkInput,
};
use vgdi_types::{
    BackSpec, BleedMode, Duplex, FillOrder, InnerBleed, JobSpec, Manual, NUp, PerfectBound, Rect,
    ScaleMode, Scheme, StepRepeat, SurfaceSide, WorkStyle, SCHEMA_ID,
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
    let layout = match job.scheme {
        Scheme::SaddleStitch(_) | Scheme::PerfectBound(_) => SurfaceLayout::Folded,
        Scheme::StepRepeat(_) => SurfaceLayout::Gang,
        _ => SurfaceLayout::Independent,
    };
    let booklet = layout == SurfaceLayout::Folded;
    // Collation back-step marks order *gathered* signatures, so they apply to perfect binding only —
    // a saddle-stitch booklet is a single nested signature with nothing to collate.
    let per_sig = sheets_per_signature(job);
    // `work_style` drives a back surface only on the gang/N-up paths; on tumble/perfector it moves the
    // gripper to the opposite edge, so that back surface's furniture parks at the top.
    let ws = job.sheet.work_style;
    let work_style_drives = matches!(job.scheme, Scheme::NUp(_) | Scheme::StepRepeat(_));

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
            let gripper_edge = if work_style_drives
                && surface.side == SurfaceSide::Back
                && matches!(ws, WorkStyle::WorkAndTumble | WorkStyle::Perfector)
            {
                GripperEdge::Top
            } else {
                GripperEdge::Bottom
            };
            let input = SurfaceMarkInput {
                cells: &placed,
                fold_lines: &fold_lines,
                sheet: sheet_rect,
                gripper: job.sheet.gripper_pt,
                gripper_edge,
                layout,
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

    // Optional duplex back (M2). `work_style` only takes effect when a back is configured.
    let ws = job.sheet.work_style;
    let back_src = resolve_back(sources, n.back.as_ref(), src.pages.len())?;

    let mut sheets = Vec::new();
    let mut front = Vec::new();
    let mut back = Vec::new();
    for page in 0..src.pages.len() {
        let (trim, bleed, rot, cs) = validate_page(src, page)?;
        let slot = front.len();
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

        // Paired back cell: place the back page into the work-style-reflected cell — upright for
        // turn/tumble, rotated 180° for perfector (never mirrored). Equal geometry (validated) ⇒ the
        // bleed gutter already checked on the front holds for the back.
        if let Some(bsrc) = back_src {
            let (bt, bb, brot, bcs) = validate_page(bsrc, page)?;
            require_back_geometry(&bsrc.id, page, trim, bleed, bt, bb)?;
            let bcell = work_style_reflect(cell, ws, sw, sh);
            let bp = place_best(
                bt,
                brot,
                bcell,
                n.scale,
                n.rotate_to_fit,
                work_style_content_flip(ws),
            );
            let bclip = back_clip(&p.ctm, clip, &bp.ctm, ws, sw, sh);
            back.push(make_cell(&bsrc.id, page, bp.ctm, bclip, bt, bb, bcs));
        }

        front.push(make_cell(&src.id, page, p.ctm, clip, trim, bleed, cs));
        if front.len() == per_sheet {
            sheets.push(duplex_sheet(
                sw,
                sh,
                std::mem::take(&mut front),
                std::mem::take(&mut back),
            ));
        }
    }
    if !front.is_empty() {
        sheets.push(duplex_sheet(sw, sh, front, back));
    }
    Ok(ImpositionPlan { sheets })
}

// ------------------------------------------------------------- M2 work styles / duplex back (gang)

/// The work-style **position transform** `T`: where the back cell paired with a front cell at `cell`
/// lands on the same sheet (SPEC §9). A *position* reflection only — the back content is placed
/// upright (turn/tumble) or rotated 180° (perfector, see [`work_style_content_flip`]); content is
/// never mirrored, so every back CTM keeps det > 0.
fn work_style_reflect(cell: Rect, ws: WorkStyle, sheet_w: f64, sheet_h: f64) -> Rect {
    match ws {
        // Sheetwise: the back is imposed on its own independent grid → same positions as the front.
        WorkStyle::Sheetwise => cell,
        // Work-and-turn: reflect about the vertical centreline; gripper stays on the same edge.
        WorkStyle::WorkAndTurn => reflect_x(cell, sheet_w / 2.0),
        // Work-and-tumble: reflect about the horizontal centreline; gripper moves to the tail edge.
        WorkStyle::WorkAndTumble => reflect_y(cell, sheet_h / 2.0),
        // Perfector: 180° rotation about the sheet centre = reflect both axes; gripper opposite edge.
        WorkStyle::Perfector => reflect_y(reflect_x(cell, sheet_w / 2.0), sheet_h / 2.0),
    }
}

/// Perfector images the back as the front **rotated 180°** (SPEC §9), so the back *content* flips
/// 180° as well (still det > 0 — a rotation, not a mirror). Turn/tumble keep content upright.
fn work_style_content_flip(ws: WorkStyle) -> bool {
    matches!(ws, WorkStyle::Perfector)
}

/// Resolve the optional duplex back source: it must be a declared source with the **same page count**
/// as the front (1:1 pairing). `None` when no back is configured.
fn resolve_back<'a>(
    sources: &'a [SourceInfo],
    back: Option<&BackSpec>,
    front_pages: usize,
) -> Result<Option<&'a SourceInfo>> {
    let Some(spec) = back else {
        return Ok(None);
    };
    let src = sources
        .iter()
        .find(|s| s.id == spec.source)
        .ok_or_else(|| EngineError::UnknownSource(spec.source.clone()))?;
    if src.pages.len() != front_pages {
        return Err(EngineError::BackCountMismatch {
            back: src.id.clone(),
            back_pages: src.pages.len(),
            front_pages,
        });
    }
    Ok(Some(src))
}

/// True when two rects have the same width/height (origin may differ).
fn same_size(a: Rect, b: Rect) -> bool {
    const TOL: f64 = 1e-6;
    (a.width() - b.width()).abs() < TOL && (a.height() - b.height()).abs() < TOL
}

/// v1 requires each back page's trim **and** bleed to match its paired front page's size, so the cut
/// registers and the front-derived placement/clip carries to the back unchanged. Reject otherwise.
fn require_back_geometry(
    back_id: &str,
    page: usize,
    front_trim: Rect,
    front_bleed: Rect,
    back_trim: Rect,
    back_bleed: Rect,
) -> Result<()> {
    if same_size(front_trim, back_trim) && same_size(front_bleed, back_bleed) {
        Ok(())
    } else {
        Err(EngineError::BackGeometryMismatch {
            back: back_id.to_string(),
            page,
        })
    }
}

/// The back cell's form `/BBox` (back page space): reflect the front cell's clip through the
/// work-style transform `T`, computed in sheet space and mapped back through the back CTM's inverse.
/// This is the booklet spine-safe-clip pattern generalised to the gang back, so inner-bleed creep and
/// asymmetric clips carry over; for `Sheetwise` (identity `T`) it re-expresses the clip in back space.
fn back_clip(
    front_ctm: &Matrix,
    front_clip: Rect,
    back_ctm: &Matrix,
    ws: WorkStyle,
    sheet_w: f64,
    sheet_h: f64,
) -> Rect {
    let sheet = transform_rect_bounds(front_ctm, front_clip);
    let reflected = work_style_reflect(sheet, ws, sheet_w, sheet_h);
    transform_rect_bounds(&back_ctm.inverse(), reflected)
}

/// Emit a sheet with a front surface and, when `back` is non-empty, a reflected back surface.
fn duplex_sheet(w: f64, h: f64, front: Vec<Cell>, back: Vec<Cell>) -> PlannedSheet {
    let mut surfaces = vec![Surface {
        side: SurfaceSide::Front,
        cells: front,
        marks: MarkPlan::default(),
    }];
    if !back.is_empty() {
        surfaces.push(Surface {
            side: SurfaceSide::Back,
            cells: back,
            marks: MarkPlan::default(),
        });
    }
    PlannedSheet {
        width: w,
        height: h,
        surfaces,
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

/// Fixed scale factor for a step (Fit is meaningless for a fixed-step tile → 100%).
fn step_scale(scale: ScaleMode) -> f64 {
    match scale {
        ScaleMode::None | ScaleMode::Fit => 1.0,
        ScaleMode::Fixed(f) => f,
    }
}

/// How many identical cards of footprint `card` fit in `usable` at centre-to-centre pitch `step`,
/// then cap by `max` (0 = no cap). The block of `n` cards spans `(n-1)·step + card` (the outer
/// edges always show the full card box), so `n ≤ (usable − card)/step + 1`. With no creep `step =
/// card + space`, which reduces to the plain tight-pack count.
fn fit_count(usable: f64, card: f64, step: f64, max: u32) -> usize {
    if card <= 0.0 || step <= 0.0 {
        return 0;
    }
    let fit = ((usable - card) / step + 1.0).floor().max(0.0) as usize;
    if max == 0 {
        fit
    } else {
        fit.min(max as usize)
    }
}

/// Inner-bleed creep for one axis, given the two opposing per-side bleeds (`b_lo` on the
/// low-coordinate edge, `b_hi` on the high-coordinate edge, both sheet space). Returns how far to
/// pull **each** neighbour-facing edge in from full bleed: `(creep_lo, creep_hi)`.
///
/// The seam where two cards' clipped bleeds meet must sit on the cut line, which is centred between
/// the two trims — so each card keeps the **same** retained bleed (`band/2`) on either side of a
/// boundary, regardless of how the trim sits within the bleed box. With a symmetric bleed this is the
/// plain `bleed·(1−f)` creep on both edges; with an off-centre trim (asymmetric bleed) the wider edge
/// is cropped more so the seam still lands on the knife. `retained` is capped to the smaller available
/// bleed so neither edge crops past its own bleed, and non-finite parameters fall back to `Full`.
fn inner_creeps(spec: InnerBleed, b_lo: f64, b_hi: f64) -> (f64, f64) {
    let full = b_lo + b_hi;
    if full <= 0.0 {
        return (0.0, 0.0); // no bleed to creep (NoBleed tiling)
    }
    let band = match spec {
        InnerBleed::Full => return (0.0, 0.0),
        InnerBleed::Fraction(f) if f.is_finite() => full * f.clamp(0.0, 1.0),
        InnerBleed::CombinedPt(t) if t.is_finite() => t.max(0.0),
        // NaN / inf -> treat as Full (no creep) rather than silently cropping everything.
        InnerBleed::Fraction(_) | InnerBleed::CombinedPt(_) => return (0.0, 0.0),
    };
    // Retained per edge = half the (clamped) combined band, never more than the smaller available
    // bleed so the seam can stay centred between the trims.
    let retained = (band.min(full) / 2.0).min(b_lo).min(b_hi);
    ((b_lo - retained).max(0.0), (b_hi - retained).max(0.0))
}

/// Per-edge creep amounts for a gang cell's four inner-facing edges (sheet space).
#[derive(Clone, Copy, Debug)]
struct Creep {
    left: f64,
    right: f64,
    bottom: f64,
    top: f64,
}

impl Creep {
    fn any(&self) -> bool {
        self.left > 0.0 || self.right > 0.0 || self.bottom > 0.0 || self.top > 0.0
    }
}

/// Asymmetric form `/BBox` (page space) for an inner-bleed-creep gang cell: full bleed on the gang's
/// **outer** edges, pulled in (per edge) on edges that face a neighbour, so adjacent cards' clipped
/// bleeds meet at the centred cut line instead of overlapping. Built in sheet space from the placed
/// bleed-box `cell` (the placement anchors the bleed box onto it), then mapped back to page space
/// through the inverse CTM so page `/Rotate` is handled. Row 0 is at the top.
fn inner_bleed_clip(
    ctm: &Matrix,
    cell: Rect,
    creep: Creep,
    r: usize,
    c: usize,
    rows: usize,
    cols: usize,
) -> Rect {
    let mut s = cell;
    if c > 0 {
        s.llx += creep.left; // left neighbour
    }
    if c + 1 < cols {
        s.urx -= creep.right; // right neighbour
    }
    if r + 1 < rows {
        s.lly += creep.bottom; // row below
    }
    if r > 0 {
        s.ury -= creep.top; // row above
    }
    transform_rect_bounds(&ctm.inverse(), s)
}

/// Step & Repeat: gang one design per sheet, tiled **tight** by the *card box* — the **bleed box**
/// when `Bleed` (default), so neighbours tile bleed-to-bleed: their bleeds meet (no overlap, no
/// hairline) and each card's trim sits 3 mm inside, leaving the proper bleed band between trims (the
/// two trim crop marks end up one bleed apart, ≈6 mm for a 3 mm bleed). `NoBleed` tiles by the trim.
/// `h_space_pt`/`v_space_pt` add a gap *between card boxes* (default 0). The block is centred in the
/// imageable area; count auto-fits unless capped by `max_rows`/`max_cols`. Crop marks sit on the
/// gang perimeter at the trim cut lines (see `attach_marks` → Gang layout).
///
/// Optional **inner-bleed creep** ([`StepRepeat::inner_bleed`]) crops the shared bleed on
/// neighbour-facing edges so the cards step closer (the pitch shrinks by the cropped amount, fitting
/// more per sheet) while the outer perimeter keeps full bleed and each cut line stays centred
/// between its two trims. The crop marks follow the moved trims automatically.
fn plan_step_repeat(
    job: &JobSpec,
    sources: &[SourceInfo],
    sr: &StepRepeat,
) -> Result<ImpositionPlan> {
    let src = primary(sources)?;
    if src.pages.is_empty() {
        return Err(EngineError::EmptySource { id: src.id.clone() });
    }
    let (sw, sh) = (job.sheet.size_pt[0], job.sheet.size_pt[1]);
    let (gripper, margin) = (job.sheet.gripper_pt, job.sheet.margin_pt);
    let usable_w = sw - 2.0 * margin;
    let usable_h = sh - gripper - 2.0 * margin;
    let s = step_scale(sr.scale);

    // Optional duplex back (M2). `work_style` only takes effect when a back is configured.
    let ws = job.sheet.work_style;
    let back_src = resolve_back(sources, sr.back.as_ref(), src.pages.len())?;

    let mut sheets = Vec::new();
    for page in 0..src.pages.len() {
        let (trim, bleed, rot, cs) = validate_page(src, page)?;
        // The card box each repeat occupies (and shows): the bleed box (bleeds meet) or the trim.
        let card = match sr.bleed_mode {
            BleedMode::Bleed => bleed,
            BleedMode::NoBleed => trim,
        };
        // Placed card-box footprint (post-rotation, scaled).
        let (cw, ch) = if rot % 180 == 90 {
            (card.height(), card.width())
        } else {
            (card.width(), card.height())
        };
        let (pw, ph) = (cw * s, ch * s);
        // Probe placement (a cell exactly the card footprint) to read the per-edge bleed in sheet
        // space, rotation-correct — the trim's offset within the card box may be asymmetric, so each
        // side is measured independently rather than assumed symmetric.
        let probe = place_best(
            card,
            rot,
            Rect::new(0.0, 0.0, pw, ph),
            sr.scale,
            false,
            false,
        );
        let pt = transform_rect_bounds(&probe.ctm, trim);
        let (bl, br) = ((pt.llx).max(0.0), (pw - pt.urx).max(0.0)); // left / right bleed
        let (bb, bt) = ((pt.lly).max(0.0), (ph - pt.ury).max(0.0)); // bottom / top bleed
        let (cl, cr) = inner_creeps(sr.inner_bleed, bl, br);
        let (cb, ct) = inner_creeps(sr.inner_bleed, bb, bt);
        let creep = Creep {
            left: cl,
            right: cr,
            bottom: cb,
            top: ct,
        };

        // Inner-bleed creep shortens the pitch by the cropped amount on both inner edges of a
        // boundary, so the cards step closer; the outer perimeter still shows full bleed (block spans
        // `(n-1)·step + card`). With `InnerBleed::Full` (or no bleed) creep is 0 and the pitch is
        // `card + space`.
        let step_w = pw - (cl + cr) + sr.h_space_pt;
        let step_h = ph - (cb + ct) + sr.v_space_pt;

        let cols = fit_count(usable_w, pw, step_w, sr.max_cols);
        let rows = fit_count(usable_h, ph, step_h, sr.max_rows);
        if cols == 0 {
            return Err(EngineError::SheetTooSmall { axis: "x" });
        }
        if rows == 0 {
            return Err(EngineError::SheetTooSmall { axis: "y" });
        }

        let block_w = (cols as f64 - 1.0) * step_w + pw;
        let block_h = (rows as f64 - 1.0) * step_h + ph;
        let ox = margin + (usable_w - block_w) / 2.0;
        let oy = gripper + margin + (usable_h - block_h) / 2.0;

        // Resolve the paired back page once per gang (equal geometry ⇒ identical card box & tiling).
        let back_page = match back_src {
            Some(bsrc) => {
                let (bt, bb, brot, bcs) = validate_page(bsrc, page)?;
                require_back_geometry(&bsrc.id, page, trim, bleed, bt, bb)?;
                let bcard = match sr.bleed_mode {
                    BleedMode::Bleed => bb,
                    BleedMode::NoBleed => bt,
                };
                Some((bsrc, bt, bb, brot, bcs, bcard))
            }
            None => None,
        };

        let creeping = creep.any();
        let mut cells = Vec::new();
        let mut back_cells = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                let cx = ox + c as f64 * step_w;
                let cy = oy + (rows - 1 - r) as f64 * step_h; // row 0 at the top
                let cell = Rect::new(cx, cy, cx + pw, cy + ph);
                // Anchor the card box on the cell; the trim (inset by the bleed) lands inside it.
                let p = place_best(card, rot, cell, sr.scale, false, false);
                // Clip the form: full bleed by default, or asymmetric (inner edges cropped) when
                // creeping. Full creep reproduces `card` exactly, keeping byte-identical output.
                let clip = if creeping {
                    inner_bleed_clip(&p.ctm, cell, creep, r, c, rows, cols)
                } else {
                    card
                };
                // Paired back card: place into the reflected cell (upright for turn/tumble, 180° for
                // perfector); the clip (incl. creep) follows the front through `T` into back page space.
                if let Some((bsrc, bt, bb, brot, bcs, bcard)) = back_page {
                    let bcell = work_style_reflect(cell, ws, sw, sh);
                    let bp = place_best(
                        bcard,
                        brot,
                        bcell,
                        sr.scale,
                        false,
                        work_style_content_flip(ws),
                    );
                    let bclip = back_clip(&p.ctm, clip, &bp.ctm, ws, sw, sh);
                    back_cells.push(make_cell(&bsrc.id, page, bp.ctm, bclip, bt, bb, bcs));
                }
                cells.push(make_cell(&src.id, page, p.ctm, clip, trim, bleed, cs));
            }
        }
        sheets.push(duplex_sheet(sw, sh, cells, back_cells));
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
            back: None,
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

    fn step_repeat(max_rows: u32, max_cols: u32, space: f64, mode: BleedMode) -> Scheme {
        step_repeat_creep(max_rows, max_cols, space, mode, InnerBleed::Full)
    }

    fn step_repeat_creep(
        max_rows: u32,
        max_cols: u32,
        space: f64,
        mode: BleedMode,
        inner_bleed: InnerBleed,
    ) -> Scheme {
        Scheme::StepRepeat(StepRepeat {
            max_rows,
            max_cols,
            h_space_pt: space,
            v_space_pt: space,
            bleed_mode: mode,
            inner_bleed,
            scale: ScaleMode::None,
            back: None,
        })
    }

    #[test]
    fn step_repeat_one_sheet_per_page_capped_grid() {
        let p = plan(&job(step_repeat(3, 4, 10.0, BleedMode::Bleed)), &src(2)).unwrap();
        assert_eq!(p.sheets.len(), 2, "one sheet per source page");
        assert_eq!(
            p.sheets[0].surfaces[0].cells.len(),
            12,
            "capped to 3x4 = 12"
        );
    }

    #[test]
    fn step_repeat_auto_fits_when_uncapped() {
        // 0/0 = fit as many as the sheet allows; Bleed mode steps by the 190×190 *bleed* box on an
        // 800×800 sheet with a 0.1pt gap → floor((800−190)/190.1 + 1) = 4 each way (pitch = card +
        // space when not creeping).
        let p = plan(&job(step_repeat(0, 0, 0.1, BleedMode::Bleed)), &src(1)).unwrap();
        assert_eq!(p.cell_count(), 16, "4x4 auto-fit");
    }

    #[test]
    fn step_repeat_tiles_by_bleed_box_with_trim_inset() {
        // Bleed mode tiles by the 190-wide *bleed* box: block = 2*190+10 = 390, centred on the 800
        // sheet → bleed boxes at 205 and 405 (meeting with the 10pt gap). Each trim sits 5pt inside
        // its bleed, so the two cut lines end up gap + 2×bleed = 20pt apart; clip = the bleed box.
        let p = plan(&job(step_repeat(1, 2, 10.0, BleedMode::Bleed)), &src(1)).unwrap();
        let cells = &p.sheets[0].surfaces[0].cells;
        assert_eq!(cells.len(), 2);
        assert_eq!(
            cells[0].bbox,
            Rect::new(5.0, 5.0, 195.0, 195.0),
            "clip = bleed box"
        );
        let placed = |c: &Cell, r: Rect| transform_rect_bounds(&c.ctm, r);
        let mut by_x: Vec<&Cell> = cells.iter().collect();
        by_x.sort_by(|a, b| {
            placed(a, a.bleed)
                .llx
                .partial_cmp(&placed(b, b.bleed).llx)
                .unwrap()
        });
        let (lb, rb) = (
            placed(by_x[0], by_x[0].bleed),
            placed(by_x[1], by_x[1].bleed),
        );
        assert!((lb.llx - 205.0).abs() < 1e-6, "bleed block centred at 205");
        assert!(
            (rb.llx - lb.urx - 10.0).abs() < 1e-6,
            "bleed boxes tile bleed-to-bleed with the gap"
        );
        let (lt, rt) = (placed(by_x[0], by_x[0].trim), placed(by_x[1], by_x[1].trim));
        assert!(
            (rt.llx - lt.urx - 20.0).abs() < 1e-6,
            "cut lines separated by gap + 2× bleed (proper bleed band)"
        );
    }

    #[test]
    fn step_repeat_no_bleed_clips_to_trim() {
        let p = plan(&job(step_repeat(1, 1, 0.0, BleedMode::NoBleed)), &src(1)).unwrap();
        assert_eq!(
            p.sheets[0].surfaces[0].cells[0].bbox,
            Rect::new(10.0, 10.0, 190.0, 190.0)
        );
    }

    #[test]
    fn step_repeat_rejects_card_larger_than_sheet() {
        let mut j = job(step_repeat(0, 0, 0.0, BleedMode::Bleed));
        j.sheet.size_pt = [100.0, 100.0]; // smaller than the 180×180 trim
        assert!(matches!(
            plan(&j, &src(1)).unwrap_err(),
            EngineError::SheetTooSmall { .. }
        ));
    }

    #[test]
    fn step_repeat_crop_marks_stay_on_the_gang_perimeter() {
        // Gang crop marks must never originate inside the block — only on the outer perimeter.
        let mut j = job(step_repeat(2, 2, 0.0, BleedMode::Bleed));
        j.marks.crop = Some(CropMarks::default());
        let p = plan(&j, &src(1)).unwrap();
        let surf = &p.sheets[0].surfaces[0];
        let gt = surf
            .cells
            .iter()
            .map(|c| transform_rect_bounds(&c.ctm, c.trim))
            .reduce(|a, b| {
                Rect::new(
                    a.llx.min(b.llx),
                    a.lly.min(b.lly),
                    a.urx.max(b.urx),
                    a.ury.max(b.ury),
                )
            })
            .unwrap();
        let mut ticks = 0;
        for prim in &surf.marks.primitives {
            if let crate::marks::MarkPrimitive::Line { from, to, .. } = prim {
                for (x, y) in [from, to] {
                    let inside = *x > gt.llx + 1e-6
                        && *x < gt.urx - 1e-6
                        && *y > gt.lly + 1e-6
                        && *y < gt.ury - 1e-6;
                    assert!(!inside, "crop mark inside the gang at ({x},{y})");
                }
                ticks += 1;
            }
        }
        assert!(ticks > 0, "perimeter crop marks were emitted");
    }

    /// A single-page source with explicit trim/bleed/media boxes, for step-&-repeat creep geometry.
    fn card_src(trim: Rect, bleed: Rect, media: Rect) -> Vec<SourceInfo> {
        vec![SourceInfo {
            id: "card".into(),
            pages: vec![PageGeometry {
                boxes: PageBoxes {
                    media,
                    crop: None,
                    trim: Some(trim),
                    art: None,
                    bleed: Some(bleed),
                },
                rotate: 0,
                group_cs: GroupCs::DeviceCmyk,
            }],
        }]
    }

    fn card_job(scheme: Scheme, sheet_w: f64, sheet_h: f64) -> JobSpec {
        let mut j = job(scheme);
        j.sheet.size_pt = [sheet_w, sheet_h];
        j
    }

    #[test]
    fn step_repeat_creep_increases_yield() {
        // 80pt trim + 10pt bleed/side = 100pt card box. On a 360pt sheet, full-bleed tiling fits 3
        // (block 3×100 = 300; a 4th needs 400). Creeping the inner bleed to zero shrinks the pitch
        // to the 80pt trim → 4 fit each way: 3×3 = 9 becomes 4×4 = 16.
        let trim = Rect::new(10.0, 10.0, 90.0, 90.0);
        let bleed = Rect::new(0.0, 0.0, 100.0, 100.0);
        let media = Rect::new(0.0, 0.0, 100.0, 100.0);
        let sources = card_src(trim, bleed, media);

        let full = plan(
            &card_job(step_repeat(0, 0, 0.0, BleedMode::Bleed), 360.0, 360.0),
            &sources,
        )
        .unwrap();
        assert_eq!(full.cell_count(), 9, "full bleed: 3×3");

        let crept = plan(
            &card_job(
                step_repeat_creep(0, 0, 0.0, BleedMode::Bleed, InnerBleed::Fraction(0.0)),
                360.0,
                360.0,
            ),
            &sources,
        )
        .unwrap();
        assert_eq!(crept.cell_count(), 16, "inner bleed cropped to zero: 4×4");
    }

    #[test]
    fn step_repeat_creep_centers_cut_and_keeps_outer_bleed_full() {
        // 1×2 gang, 80pt trim + 10pt bleed/side, creep to half bleed (keep 5pt of each inner bleed).
        // The inner edges crop 5pt; the outer edges keep full bleed; the cut line lands centred
        // between the two trims.
        let trim = Rect::new(10.0, 10.0, 90.0, 90.0);
        let bleed = Rect::new(0.0, 0.0, 100.0, 100.0);
        let media = Rect::new(0.0, 0.0, 100.0, 100.0);
        let sources = card_src(trim, bleed, media);
        let p = plan(
            &card_job(
                step_repeat_creep(1, 2, 0.0, BleedMode::Bleed, InnerBleed::Fraction(0.5)),
                400.0,
                200.0,
            ),
            &sources,
        )
        .unwrap();
        let cells = &p.sheets[0].surfaces[0].cells;
        assert_eq!(cells.len(), 2);
        let mut by_x: Vec<&Cell> = cells.iter().collect();
        by_x.sort_by(|a, b| a.bbox.llx.partial_cmp(&b.bbox.llx).unwrap());
        let (l, r) = (by_x[0], by_x[1]);

        // Left card: outer (left) edge keeps full bleed (page-space llx 0), inner (right) cropped 5pt.
        assert!((l.bbox.llx - 0.0).abs() < 1e-6, "left outer bleed full");
        assert!(
            (l.bbox.urx - 95.0).abs() < 1e-6,
            "left inner bleed cropped to 5pt"
        );
        // Right card: inner (left) cropped 5pt, outer (right) keeps full bleed (page-space urx 100).
        assert!(
            (r.bbox.llx - 5.0).abs() < 1e-6,
            "right inner bleed cropped to 5pt"
        );
        assert!((r.bbox.urx - 100.0).abs() < 1e-6, "right outer bleed full");

        // Trims: interior gap halves (10pt) vs full bleed (20pt); the cut line sits centred between.
        let lt = transform_rect_bounds(&l.ctm, l.trim);
        let rt = transform_rect_bounds(&r.ctm, r.trim);
        assert!(
            (rt.llx - lt.urx - 10.0).abs() < 1e-6,
            "combined inner bleed halved to 10pt"
        );
        // Clipped inner bleeds meet at the midpoint between the trims.
        let lb = transform_rect_bounds(&l.ctm, l.bbox);
        let rb = transform_rect_bounds(&r.ctm, r.bbox);
        let cut = (lt.urx + rt.llx) / 2.0;
        assert!(
            (lb.urx - cut).abs() < 1e-6,
            "left clip meets the centred cut"
        );
        assert!(
            (rb.llx - cut).abs() < 1e-6,
            "right clip meets the centred cut"
        );
    }

    #[test]
    fn step_repeat_creep_asymmetric_bleed_keeps_seam_on_centred_cut() {
        // Off-centre trim inside the bleed box: 5pt bleed on the left, 15pt on the right (bL≠bR). The
        // seam where two clipped bleeds meet must still land on the knife — centred between the two
        // trims — so the wider edge is cropped more. (Averaging the two bleeds would offset the seam
        // by (bR−bL)/2 and paint one card's ink past the cut.)
        let trim = Rect::new(5.0, 10.0, 85.0, 90.0); // bL=5, bR=15, bB=bT=10
        let bleed = Rect::new(0.0, 0.0, 100.0, 100.0);
        let media = Rect::new(0.0, 0.0, 100.0, 100.0);
        let sources = card_src(trim, bleed, media);
        let p = plan(
            &card_job(
                step_repeat_creep(1, 2, 0.0, BleedMode::Bleed, InnerBleed::Fraction(0.5)),
                400.0,
                200.0,
            ),
            &sources,
        )
        .unwrap();
        let cells = &p.sheets[0].surfaces[0].cells;
        assert_eq!(cells.len(), 2);
        // Sort by placed (sheet-space) position: the asymmetric clip leaves both cards' page-space
        // bbox.llx at 0 (the right card keeps its full small left bleed), so page space won't order.
        let mut by_x: Vec<&Cell> = cells.iter().collect();
        by_x.sort_by(|a, b| {
            transform_rect_bounds(&a.ctm, a.trim)
                .llx
                .partial_cmp(&transform_rect_bounds(&b.ctm, b.trim).llx)
                .unwrap()
        });
        let (l, r) = (by_x[0], by_x[1]);

        let lt = transform_rect_bounds(&l.ctm, l.trim);
        let rt = transform_rect_bounds(&r.ctm, r.trim);
        let lb = transform_rect_bounds(&l.ctm, l.bbox);
        let rb = transform_rect_bounds(&r.ctm, r.bbox);
        let cut = (lt.urx + rt.llx) / 2.0;
        // The seam sits exactly on the centred cut — neither card's ink crosses it.
        assert!(
            (lb.urx - cut).abs() < 1e-6,
            "left clip meets the centred cut, not offset by (bR−bL)/2: clip_r={} cut={}",
            lb.urx,
            cut
        );
        assert!(
            (rb.llx - cut).abs() < 1e-6,
            "right clip meets the centred cut: clip_l={} cut={}",
            rb.llx,
            cut
        );
        assert!(
            lb.urx <= cut + 1e-6,
            "left card must not paint past the cut"
        );
        assert!(
            rb.llx >= cut - 1e-6,
            "right card must not paint past the cut"
        );
        // Outer edges keep full bleed (page space): left card's left = 0, right card's right = 100.
        assert!((l.bbox.llx - 0.0).abs() < 1e-6, "left outer bleed full");
        assert!((r.bbox.urx - 100.0).abs() < 1e-6, "right outer bleed full");
    }

    #[test]
    fn inner_creeps_non_finite_falls_back_to_full() {
        // NaN / inf parameters must crop nothing (Full), never silently remove the whole inner bleed.
        for spec in [
            InnerBleed::Fraction(f64::NAN),
            InnerBleed::Fraction(f64::INFINITY),
            InnerBleed::CombinedPt(f64::NAN),
            InnerBleed::CombinedPt(f64::INFINITY),
        ] {
            assert_eq!(inner_creeps(spec, 8.5, 8.5), (0.0, 0.0), "{spec:?} -> Full");
        }
        // Sanity: finite specs still creep, and symmetric bleed creeps both edges equally.
        assert_eq!(
            inner_creeps(InnerBleed::Fraction(0.5), 8.5, 8.5),
            (4.25, 4.25)
        );
        assert_eq!(inner_creeps(InnerBleed::Full, 8.5, 8.5), (0.0, 0.0));
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

    // ---------------------------------------------------- M2 work styles / duplex back (Phase 1)

    /// `body` + `back` sources, identical geometry (so the v1 equal-trim constraint holds).
    fn src_two(n: usize) -> Vec<SourceInfo> {
        let mk = |id: &str| SourceInfo {
            id: id.into(),
            pages: (0..n)
                .map(|_| geom(Some(Rect::new(10.0, 10.0, 190.0, 190.0)), 0))
                .collect(),
        };
        vec![mk("body"), mk("back")]
    }

    fn nup_back(rows: u32, cols: u32, bleed: BleedMode) -> Scheme {
        Scheme::NUp(NUp {
            rows,
            cols,
            fill: FillOrder::RowMajor,
            scale: ScaleMode::Fit,
            gutter_pt: 0.0,
            rotate_to_fit: false,
            bleed_mode: bleed,
            back: Some(BackSpec {
                source: "back".into(),
            }),
        })
    }

    fn step_repeat_back(mode: BleedMode) -> Scheme {
        Scheme::StepRepeat(StepRepeat {
            max_rows: 2,
            max_cols: 2,
            h_space_pt: 0.0,
            v_space_pt: 0.0,
            bleed_mode: mode,
            inner_bleed: InnerBleed::Full,
            scale: ScaleMode::None,
            back: Some(BackSpec {
                source: "back".into(),
            }),
        })
    }

    fn det(m: &Matrix) -> f64 {
        m.a * m.d - m.c * m.b
    }

    fn rect_approx(a: Rect, b: Rect) {
        for (x, y) in [
            (a.llx, b.llx),
            (a.lly, b.lly),
            (a.urx, b.urx),
            (a.ury, b.ury),
        ] {
            assert!((x - y).abs() < 1e-9, "rect mismatch: {a:?} vs {b:?}");
        }
    }

    fn placed_trim(c: &Cell) -> Rect {
        crate::geom::transform_rect_bounds(&c.ctm, c.trim)
    }

    #[test]
    fn nup_back_sheetwise_places_back_at_same_positions() {
        // Sheetwise: the back is imposed on its own independent grid → back cell (r,c) sits at the
        // same sheet position as front cell (r,c). Two surfaces (Front, Back) on one sheet.
        let p = plan(&job(nup_back(2, 2, BleedMode::NoBleed)), &src_two(4)).unwrap();
        assert_eq!(p.sheets.len(), 1);
        let s = &p.sheets[0];
        assert_eq!(s.surfaces.len(), 2);
        assert_eq!(s.surfaces[0].side, SurfaceSide::Front);
        assert_eq!(s.surfaces[1].side, SurfaceSide::Back);
        assert_eq!(s.surfaces[1].cells.len(), 4);
        for (f, b) in s.surfaces[0].cells.iter().zip(&s.surfaces[1].cells) {
            assert_eq!(b.source_id, "back");
            assert_eq!(placed_trim(f), placed_trim(b));
        }
    }

    #[test]
    fn nup_back_work_and_turn_reflects_positions_and_is_involutive() {
        // Work-and-turn: back trim = front trim reflected about the vertical centreline (x = W/2),
        // and reflecting again recovers the front (T∘T = identity).
        let mut j = job(nup_back(2, 2, BleedMode::NoBleed));
        j.sheet.work_style = WorkStyle::WorkAndTurn;
        let axis = j.sheet.size_pt[0] / 2.0;
        let p = plan(&j, &src_two(4)).unwrap();
        let s = &p.sheets[0];
        for (f, b) in s.surfaces[0].cells.iter().zip(&s.surfaces[1].cells) {
            let fp = placed_trim(f);
            let bp = placed_trim(b);
            rect_approx(bp, crate::geom::reflect_x(fp, axis));
            rect_approx(crate::geom::reflect_x(bp, axis), fp);
        }
    }

    #[test]
    fn gang_back_work_and_turn_reflects_positions() {
        let mut j = job(step_repeat_back(BleedMode::Bleed));
        j.sheet.work_style = WorkStyle::WorkAndTurn;
        let axis = j.sheet.size_pt[0] / 2.0;
        let p = plan(&j, &src_two(1)).unwrap();
        let s = &p.sheets[0];
        assert_eq!(s.surfaces.len(), 2);
        assert!(!s.surfaces[1].cells.is_empty());
        for (f, b) in s.surfaces[0].cells.iter().zip(&s.surfaces[1].cells) {
            rect_approx(placed_trim(b), crate::geom::reflect_x(placed_trim(f), axis));
        }
    }

    #[test]
    fn back_content_ctm_keeps_positive_determinant() {
        // SPEC §9: positions reflect (and perfector rotates 180°), content never mirrors → every
        // cell CTM has det > 0 for all four work styles.
        for ws in [
            WorkStyle::Sheetwise,
            WorkStyle::WorkAndTurn,
            WorkStyle::WorkAndTumble,
            WorkStyle::Perfector,
        ] {
            for scheme in [
                nup_back(2, 2, BleedMode::NoBleed),
                step_repeat_back(BleedMode::Bleed),
            ] {
                let mut j = job(scheme);
                j.sheet.work_style = ws;
                let p = plan(&j, &src_two(1)).unwrap();
                for sheet in &p.sheets {
                    for surface in &sheet.surfaces {
                        for cell in &surface.cells {
                            assert!(det(&cell.ctm) > 0.0, "det must stay positive (no mirror)");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn nup_back_work_and_tumble_reflects_about_horizontal_centreline() {
        let mut j = job(nup_back(2, 2, BleedMode::NoBleed));
        j.sheet.work_style = WorkStyle::WorkAndTumble;
        let axis = j.sheet.size_pt[1] / 2.0;
        let p = plan(&j, &src_two(4)).unwrap();
        let s = &p.sheets[0];
        for (f, b) in s.surfaces[0].cells.iter().zip(&s.surfaces[1].cells) {
            rect_approx(placed_trim(b), crate::geom::reflect_y(placed_trim(f), axis));
        }
    }

    #[test]
    fn nup_back_perfector_rotates_180_in_position_and_content() {
        let mut j = job(nup_back(2, 2, BleedMode::NoBleed));
        j.sheet.work_style = WorkStyle::Perfector;
        let (wax, hax) = (j.sheet.size_pt[0] / 2.0, j.sheet.size_pt[1] / 2.0);
        let p = plan(&j, &src_two(4)).unwrap();
        let s = &p.sheets[0];
        for (f, b) in s.surfaces[0].cells.iter().zip(&s.surfaces[1].cells) {
            // Position: 180° about the sheet centre = reflect both axes.
            let want = crate::geom::reflect_y(crate::geom::reflect_x(placed_trim(f), wax), hax);
            rect_approx(placed_trim(b), want);
            // Content: rotated 180° (CTM `a` < 0) — a rotation, so det stays > 0.
            assert!(b.ctm.a < 0.0, "perfector back content is rotated 180°");
            assert!(det(&b.ctm) > 0.0);
        }
    }

    #[test]
    fn moved_gripper_back_furniture_relocates_to_the_top_edge() {
        // Slug defaults to BottomLeft. On a gripper-moving back (tumble/perfector) the gripper is at
        // the tail, so the back's slug relocates to the top edge; on the front (and turn/sheetwise
        // backs, gripper unchanged) it stays at the bottom. Glyphs stay upright either way.
        for ws in [WorkStyle::WorkAndTumble, WorkStyle::Perfector] {
            let mut j = job(nup_back(2, 2, BleedMode::NoBleed));
            j.sheet.work_style = ws;
            j.marks.slug = Some(Slug::default());
            let p = plan(&j, &src_two(4)).unwrap();
            let h = p.sheets[0].height;
            let front = &p.sheets[0].surfaces[0].marks.texts;
            let back = &p.sheets[0].surfaces[1].marks.texts;
            assert!(
                !front.is_empty() && !back.is_empty(),
                "slug emitted on both surfaces"
            );
            assert!(
                front[0].y < h / 2.0,
                "front slug parks at the bottom (gripper) edge"
            );
            assert!(
                back[0].y > h / 2.0,
                "{ws:?} back slug relocates to the moved gripper edge"
            );
        }

        // Work-and-turn keeps the gripper on the same edge → the back slug stays at the bottom.
        let mut j = job(nup_back(2, 2, BleedMode::NoBleed));
        j.sheet.work_style = WorkStyle::WorkAndTurn;
        j.marks.slug = Some(Slug::default());
        let p = plan(&j, &src_two(4)).unwrap();
        let h = p.sheets[0].height;
        assert!(p.sheets[0].surfaces[1].marks.texts[0].y < h / 2.0);
    }

    #[test]
    fn work_style_is_inert_without_a_back() {
        // A job may set any work style; with no back surface it stays inert (no error, one surface).
        let mut j = job(nup(1, 1));
        j.sheet.work_style = WorkStyle::Perfector;
        let p = plan(&j, &src(1)).unwrap();
        assert_eq!(p.sheets[0].surfaces.len(), 1);
    }

    #[test]
    fn back_count_mismatch_is_rejected() {
        // front (body) has 4 pages, back has 3 → 1:1 pairing impossible.
        let mut sources = src_two(4);
        sources[1].pages.truncate(3);
        assert!(matches!(
            plan(&job(nup_back(2, 2, BleedMode::NoBleed)), &sources).unwrap_err(),
            EngineError::BackCountMismatch { .. }
        ));
    }

    #[test]
    fn back_geometry_mismatch_is_rejected() {
        // back page trim is a different size than the paired front page → cut would not register.
        let mut sources = src_two(1);
        sources[1].pages[0] = geom(Some(Rect::new(10.0, 10.0, 150.0, 150.0)), 0);
        assert!(matches!(
            plan(&job(nup_back(1, 1, BleedMode::NoBleed)), &sources).unwrap_err(),
            EngineError::BackGeometryMismatch { .. }
        ));
    }

    #[test]
    fn back_source_rotate_is_honored_upright_and_keeps_det_positive() {
        // A back page with /Rotate=90 (square trim ⇒ equal size) is placed via T: its own rotate is
        // baked into the CTM (so it prints upright), and det stays positive (rotation, not mirror).
        let mut sources = src_two(1);
        sources[1].pages[0] = geom(Some(Rect::new(10.0, 10.0, 190.0, 190.0)), 90);
        let mut j = job(nup_back(1, 1, BleedMode::NoBleed));
        j.sheet.work_style = WorkStyle::WorkAndTurn;
        let p = plan(&j, &sources).unwrap();
        let back = &p.sheets[0].surfaces[1].cells[0];
        // 90° rotation → the CTM's `a` component is ~0 (x maps from source y), unlike the unrotated
        // front, and the determinant is still > 0.
        assert!(
            back.ctm.a.abs() < 1e-9,
            "expected a 90° rotation in the back CTM"
        );
        assert!(det(&back.ctm) > 0.0);
    }
}
