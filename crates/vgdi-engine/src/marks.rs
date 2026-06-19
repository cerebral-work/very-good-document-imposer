//! Printer-mark **geometry** computation (pure). Given placed-page frames and a `MarkSet`, produce
//! the vector primitives + text runs positioned per the prepress rules (SPEC §8.6/§8.7), in **sheet
//! space**. The qpdf backend strokes these (marks in a Separation `All` colorant; SPEC §13).
//!
//! Two altitudes:
//! - [`cell_marks`] — per placed page (crop, centre, trim-outline, bleed treatment).
//! - [`plan_surface_marks`] — one surface: per-cell marks **plus** sheet-level families
//!   (registration, fold, collation, colour bar, slug, barcode) framed on the imposed content
//!   extent, not on any single page.

use crate::barcode;
use vgdi_types::{
    Barcode, BleedTreatment, CollationMarks, ColorBar, ColorBarKind, CropMarks, CropStyle,
    FoldMarks, MarkColor, MarkRegion, MarkSet, Rect, RegPositions, Slug, SlugField, SlugPosition,
    SurfaceSide, Symbology,
};

/// A positioned vector mark primitive in sheet space (points).
#[derive(Clone, Debug, PartialEq)]
pub enum MarkPrimitive {
    Line {
        from: (f64, f64),
        to: (f64, f64),
        weight: f64,
        color: MarkColor,
        /// Dash length; 0 = solid.
        dash: f64,
    },
    Rect {
        rect: Rect,
        weight: f64,
        color: MarkColor,
    },
    Circle {
        center: (f64, f64),
        radius: f64,
        weight: f64,
        color: MarkColor,
    },
    /// A solid filled rectangle (colour-bar patch, barcode bar).
    FillRect { rect: Rect, color: MarkColor },
}

/// A slug text run anchored at its lower-left, in sheet space.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkText {
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub text: String,
    pub color: MarkColor,
}

/// Everything to draw on one surface beyond the placed page cells.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkPlan {
    pub primitives: Vec<MarkPrimitive>,
    pub texts: Vec<MarkText>,
}

impl MarkPlan {
    pub fn is_empty(&self) -> bool {
        self.primitives.is_empty() && self.texts.is_empty()
    }
}

/// One placed page on a surface, in sheet space, for mark framing.
#[derive(Clone, Copy, Debug)]
pub struct PlacedCell {
    pub trim: Rect,
    pub bleed: Rect,
}

/// A fold position on the sheet (booklet spine, signature fold).
#[derive(Clone, Copy, Debug)]
pub enum FoldLine {
    /// Vertical fold at `x`, spanning `[y0, y1]`.
    Vertical { x: f64, y0: f64, y1: f64 },
    /// Horizontal fold at `y`, spanning `[x0, x1]`.
    Horizontal { y: f64, x0: f64, x1: f64 },
}

/// Per-surface context resolved by the planner (1-based counters; data, not authored copy).
#[derive(Clone, Copy, Debug)]
pub struct MarkContext<'a> {
    pub file_name: &'a str,
    pub sheet_number: usize,
    pub surface: SurfaceSide,
    /// 1-based signature number for collation back-step marks (booklets only).
    pub signature: Option<usize>,
}

/// How the placed pieces relate, which decides where cut/trim marks go (SPEC §8.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceLayout {
    /// Independent finished pieces in a grid (n-up): each page framed on its own, marks in the
    /// gutters around it.
    Independent,
    /// One folded leaf (booklet spread): marks frame the whole leaf's outer perimeter; the inner
    /// spine is a fold, not a cut, so it gets no crop ticks.
    Folded,
    /// A tight gang of identical pieces (step & repeat): crop marks only on the gang's **outer
    /// perimeter**, one tick per distinct cut line, kept clear of the bleed/print surface — never
    /// between the cards.
    Gang,
}

/// All inputs needed to plan one surface's marks.
pub struct SurfaceMarkInput<'a> {
    pub cells: &'a [PlacedCell],
    pub fold_lines: &'a [FoldLine],
    /// The sheet media rectangle, `(0, 0, w, h)`.
    pub sheet: Rect,
    pub gripper: f64,
    pub layout: SurfaceLayout,
    pub marks: &'a MarkSet,
    pub ctx: MarkContext<'a>,
}

/// Margin from the sheet edge used to park sheet-level furniture (slug, colour bar, barcode).
const FURNITURE_MARGIN: f64 = 6.0;

// ------------------------------------------------------------------------------- per-cell marks

/// Per-page marks framed on one placed page: trim outline, bleed treatment, crop, centre.
/// (Registration is sheet-level — see [`plan_surface_marks`].)
pub fn cell_marks(trim: Rect, bleed: Rect, media: Rect, marks: &MarkSet) -> Vec<MarkPrimitive> {
    let mut out = Vec::new();

    if marks.trim_outline {
        out.push(MarkPrimitive::Rect {
            rect: trim,
            weight: 0.25,
            color: MarkColor::RegistrationAll,
        });
    }

    match marks.bleed {
        BleedTreatment::Outline { weight_pt } => {
            out.push(MarkPrimitive::Rect {
                rect: bleed,
                weight: weight_pt,
                color: MarkColor::RegistrationAll,
            });
        }
        BleedTreatment::Hatched { .. } => { /* hatch fill emission is deferred */ }
        BleedTreatment::None => {}
    }

    if let Some(c) = &marks.crop {
        // Crop marks must sit outside the bleed: clamp the offset to at least the *widest* bleed
        // margin so all four corner ticks clear an asymmetric BleedBox (SPEC §8 #6), matching the
        // 4-side idiom in `plan::check_bleed_gutter`.
        let bleed_amt = (trim.llx - bleed.llx)
            .max(bleed.urx - trim.urx)
            .max(trim.lly - bleed.lly)
            .max(bleed.ury - trim.ury)
            .max(0.0);
        let offset = c.offset_pt.max(bleed_amt);
        match c.style {
            CropStyle::Classic | CropStyle::Japanese => {
                out.extend(crop_classic(
                    trim,
                    offset,
                    c.length_pt,
                    c.weight_pt,
                    &c.color,
                ));
            }
            CropStyle::FullLine => {
                out.extend(crop_full_line(trim, media, c.weight_pt, &c.color));
            }
        }
    }

    if let Some(cm) = &marks.center {
        out.extend(center_marks(trim, cm.length_pt, cm.weight_pt, &cm.color));
    }

    out
}

/// Compute mark primitives around a single trim frame (per-cell families **plus** registration).
/// Kept for direct geometry unit-testing; the surface planner calls [`cell_marks`] per cell and
/// places registration once on the content extent.
pub fn compute_marks(trim: Rect, bleed: Rect, media: Rect, marks: &MarkSet) -> Vec<MarkPrimitive> {
    let mut out = cell_marks(trim, bleed, media, marks);
    if let Some(reg) = &marks.registration {
        out.extend(registration(
            trim,
            bleed,
            reg.positions,
            reg.diameter_pt,
            reg.weight_pt,
        ));
    }
    out
}

// ----------------------------------------------------------------------------- surface planner

/// Plan all marks for one surface: per-cell families around each placed page, then sheet-level
/// families framed on the imposed content extent (union of placed trims/bleeds).
pub fn plan_surface_marks(input: &SurfaceMarkInput) -> MarkPlan {
    let marks = input.marks;
    let mut primitives = Vec::new();
    let mut texts = Vec::new();

    // The framing extent for sheet-level marks. For a folded leaf it's the union reflected about the
    // spine, so a half-blank spread frames the *full* leaf (crop on the outer perimeter only, and
    // registration in the same place it sits on full spreads — they must align when stacked).
    let mut extent = union(input.cells.iter().map(|c| c.trim));
    let mut bleed_extent = union(input.cells.iter().map(|c| c.bleed));
    if input.layout == SurfaceLayout::Folded {
        if let Some(sx) = vertical_spine_x(input.fold_lines) {
            extent = extent.map(|e| reflect_about_x(e, sx));
            bleed_extent = bleed_extent.map(|e| reflect_about_x(e, sx));
        }
    }

    // 1. Cut/trim marks (crop / centre / trim-outline / bleed) per layout:
    //    - Independent (n-up): frame each page; they're cut apart in their own gutters.
    //    - Folded (booklet): frame the whole leaf once — the spine is a fold, no crop ticks there.
    //    - Gang (step & repeat): crop marks only on the gang's outer perimeter (one per cut line),
    //      with the rest of the cut/trim families framed once on the gang extent.
    match input.layout {
        SurfaceLayout::Independent => {
            for c in input.cells {
                primitives.extend(cell_marks(c.trim, c.bleed, input.sheet, marks));
            }
        }
        SurfaceLayout::Folded => {
            if let (Some(extent), Some(bleed_extent)) = (extent, bleed_extent) {
                primitives.extend(cell_marks(extent, bleed_extent, input.sheet, marks));
            }
        }
        SurfaceLayout::Gang => {
            if let Some(crop) = &marks.crop {
                primitives.extend(gang_crop_marks(input.cells, crop));
            }
            // Centre / trim-outline / bleed treatment frame the gang extent once (no per-card crop).
            if let (Some(extent), Some(bleed_extent)) = (extent, bleed_extent) {
                let mut rest = marks.clone();
                rest.crop = None;
                primitives.extend(cell_marks(extent, bleed_extent, input.sheet, &rest));
            }
        }
    }

    if let (Some(extent), Some(bleed_extent)) = (extent, bleed_extent) {
        // 2. Registration targets — once on the sheet, just outside the content bleed.
        if let Some(reg) = &marks.registration {
            primitives.extend(registration(
                extent,
                bleed_extent,
                reg.positions,
                reg.diameter_pt,
                reg.weight_pt,
            ));
        }

        // 3. Fold marks (dashed), in the head/foot margins at each fold.
        if let Some(fm) = &marks.fold {
            primitives.extend(fold_marks(input.fold_lines, fm));
        }

        // 4. Collation back-step marks — staircase by signature number, on the spine fold.
        if let (Some(cm), Some(sig)) = (&marks.collation, input.ctx.signature) {
            primitives.extend(collation_marks(cm, input.fold_lines, bleed_extent, sig));
        }
    }

    // 5. Colour bar — process solids / tint ramp in a sheet-edge region.
    if let Some(cb) = &marks.color_bar {
        primitives.extend(color_bar(cb, input.sheet, input.gripper));
    }

    // 6. Job barcode — Code 128 bars in a sheet-edge region.
    if let Some(bc) = &marks.job_barcode {
        primitives.extend(barcode_marks(bc, input.sheet, input.gripper));
    }

    // 7. Slug — resolved info tokens as a text run.
    if let Some(slug) = &marks.slug {
        if let Some(t) = slug_text(slug, input.sheet, input.gripper, &input.ctx) {
            texts.push(t);
        }
    }

    MarkPlan { primitives, texts }
}

/// Union of a set of rectangles (the imposed content extent).
fn union(rects: impl Iterator<Item = Rect>) -> Option<Rect> {
    rects.reduce(|a, b| {
        Rect::new(
            a.llx.min(b.llx),
            a.lly.min(b.lly),
            a.urx.max(b.urx),
            a.ury.max(b.ury),
        )
    })
}

/// The x of the first vertical (spine) fold, if any.
fn vertical_spine_x(folds: &[FoldLine]) -> Option<f64> {
    folds.iter().find_map(|f| match f {
        FoldLine::Vertical { x, .. } => Some(*x),
        _ => None,
    })
}

/// Widen `r` to be symmetric about the vertical line `x` (the spine), recovering the full folded
/// leaf from a half-blank spread. Assumes left/right pages of a spread are the same width (true for
/// the uniform-page booklets M1 supports).
fn reflect_about_x(r: Rect, x: f64) -> Rect {
    Rect::new(
        r.llx.min(2.0 * x - r.urx),
        r.lly,
        r.urx.max(2.0 * x - r.llx),
        r.ury,
    )
}

/// Sort coordinates, collapse exact duplicates, then chain-cluster any remaining within `tol` into
/// their mean. With `tol = 1e-6` only identical edges merge (stacked rows/columns), so a tight
/// neighbour's distinct edge survives as a double cut; with a larger `tol` adjacent-card boundary
/// pairs merge into one shared cut line.
fn cluster_coords(it: impl Iterator<Item = f64>, tol: f64) -> Vec<f64> {
    let mut v: Vec<f64> = it.collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let mut out = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j + 1 < v.len() && v[j + 1] - v[j] <= tol {
            j += 1;
        }
        out.push(v[i..=j].iter().sum::<f64>() / (j - i + 1) as f64);
        i = j + 1;
    }
    out
}

/// Crop marks for a tight gang (step & repeat): ticks only on the gang's **outer perimeter**
/// (top/bottom for vertical cuts, left/right for horizontal), pushed clear of the gang's bleed so
/// they never touch the print surface or run between the cards (SPEC §8.6).
///
/// Whether neighbours get a **single shared** cut mark or a **double** one is decided by bleed: with
/// no bleed the cards butt and share a cut line (single); **with bleed each card's trim is cut
/// separately, so the boundary keeps both edges as a double mark** (the QI behaviour).
fn gang_crop_marks(cells: &[PlacedCell], crop: &CropMarks) -> Vec<MarkPrimitive> {
    let (Some(gt), Some(gb)) = (
        union(cells.iter().map(|c| c.trim)),
        union(cells.iter().map(|c| c.bleed)),
    ) else {
        return Vec::new();
    };
    let has_bleed = cells.iter().any(|c| {
        c.bleed.llx < c.trim.llx - 1e-6
            || c.bleed.urx > c.trim.urx + 1e-6
            || c.bleed.lly < c.trim.lly - 1e-6
            || c.bleed.ury > c.trim.ury + 1e-6
    });
    // No bleed → merge adjacent-card boundary pairs into a single shared cut. The merge window is
    // half the smallest card dimension: far wider than any sane inter-card gap, far narrower than a
    // card, so boundary pairs collapse but distinct cards never do. With bleed → keep both (double).
    let min_dim = cells
        .iter()
        .map(|c| c.trim.width().min(c.trim.height()))
        .fold(f64::INFINITY, f64::min);
    let tol = if has_bleed {
        1e-6
    } else {
        (min_dim / 2.0).max(1e-6)
    };
    let xs = cluster_coords(cells.iter().flat_map(|c| [c.trim.llx, c.trim.urx]), tol);
    let ys = cluster_coords(cells.iter().flat_map(|c| [c.trim.lly, c.trim.ury]), tol);
    // Each side's marks start past the gang bleed (+ a small gap so they clear the print surface),
    // or the requested crop offset, whichever is larger.
    let gap = 3.0;
    let off_top = crop.offset_pt.max(gb.ury - gt.ury + gap);
    let off_bot = crop.offset_pt.max(gt.lly - gb.lly + gap);
    let off_left = crop.offset_pt.max(gt.llx - gb.llx + gap);
    let off_right = crop.offset_pt.max(gb.urx - gt.urx + gap);
    let (len, w, col) = (crop.length_pt, crop.weight_pt, &crop.color);
    let mut out = Vec::new();
    for x in xs {
        out.push(line(
            (x, gt.ury + off_top),
            (x, gt.ury + off_top + len),
            w,
            col,
        ));
        out.push(line(
            (x, gt.lly - off_bot),
            (x, gt.lly - off_bot - len),
            w,
            col,
        ));
    }
    for y in ys {
        out.push(line(
            (gt.llx - off_left, y),
            (gt.llx - off_left - len, y),
            w,
            col,
        ));
        out.push(line(
            (gt.urx + off_right, y),
            (gt.urx + off_right + len, y),
            w,
            col,
        ));
    }
    out
}

// --------------------------------------------------------------------------------- primitives

fn line(from: (f64, f64), to: (f64, f64), weight: f64, color: &MarkColor) -> MarkPrimitive {
    MarkPrimitive::Line {
        from,
        to,
        weight,
        color: color.clone(),
        dash: 0.0,
    }
}

/// 8 corner ticks, offset outward from the trim corners.
fn crop_classic(t: Rect, off: f64, len: f64, w: f64, c: &MarkColor) -> Vec<MarkPrimitive> {
    vec![
        // bottom-left
        line((t.llx - off, t.lly), (t.llx - off - len, t.lly), w, c),
        line((t.llx, t.lly - off), (t.llx, t.lly - off - len), w, c),
        // bottom-right
        line((t.urx + off, t.lly), (t.urx + off + len, t.lly), w, c),
        line((t.urx, t.lly - off), (t.urx, t.lly - off - len), w, c),
        // top-left
        line((t.llx - off, t.ury), (t.llx - off - len, t.ury), w, c),
        line((t.llx, t.ury + off), (t.llx, t.ury + off + len), w, c),
        // top-right
        line((t.urx + off, t.ury), (t.urx + off + len, t.ury), w, c),
        line((t.urx, t.ury + off), (t.urx, t.ury + off + len), w, c),
    ]
}

/// 4 lines spanning the media through the trim edges.
fn crop_full_line(t: Rect, m: Rect, w: f64, c: &MarkColor) -> Vec<MarkPrimitive> {
    vec![
        line((t.llx, m.lly), (t.llx, m.ury), w, c),
        line((t.urx, m.lly), (t.urx, m.ury), w, c),
        line((m.llx, t.lly), (m.urx, t.lly), w, c),
        line((m.llx, t.ury), (m.urx, t.ury), w, c),
    ]
}

/// 4 center ticks at the midpoint of each trim edge, offset outward.
fn center_marks(t: Rect, len: f64, w: f64, c: &MarkColor) -> Vec<MarkPrimitive> {
    let (cx, cy) = t.center();
    let off = 3.0;
    vec![
        line((cx, t.lly - off), (cx, t.lly - off - len), w, c), // bottom
        line((cx, t.ury + off), (cx, t.ury + off + len), w, c), // top
        line((t.llx - off, cy), (t.llx - off - len, cy), w, c), // left
        line((t.urx + off, cy), (t.urx + off + len, cy), w, c), // right
    ]
}

/// Registration bullseyes in Separation `All`, placed just outside the bleed.
fn registration(t: Rect, b: Rect, pos: RegPositions, dia: f64, w: f64) -> Vec<MarkPrimitive> {
    let r = dia / 2.0;
    let gap = (t.llx - b.llx).max(8.0) + r; // sit beyond the bleed
    let (cx, cy) = t.center();
    let target = |x: f64, y: f64| MarkPrimitive::Circle {
        center: (x, y),
        radius: r,
        weight: w,
        color: MarkColor::RegistrationAll,
    };
    let edge_centers = vec![
        target(cx, b.lly - gap),
        target(cx, b.ury + gap),
        target(b.llx - gap, cy),
        target(b.urx + gap, cy),
    ];
    let corners = vec![
        target(b.llx - gap, b.lly - gap),
        target(b.urx + gap, b.lly - gap),
        target(b.llx - gap, b.ury + gap),
        target(b.urx + gap, b.ury + gap),
    ];
    match pos {
        RegPositions::EdgeCenters => edge_centers,
        RegPositions::Corners => corners,
        RegPositions::All => edge_centers.into_iter().chain(corners).collect(),
    }
}

/// Dashed fold marks: short ticks in the margins beyond each fold's span (head and foot, or the two
/// ends of a horizontal fold). The fold line itself is not drawn through the live area.
fn fold_marks(folds: &[FoldLine], fm: &FoldMarks) -> Vec<MarkPrimitive> {
    let mut out = Vec::new();
    let dash = |from, to| MarkPrimitive::Line {
        from,
        to,
        weight: fm.weight_pt,
        color: fm.color.clone(),
        dash: fm.dash_pt,
    };
    let len = fm.length_pt;
    for f in folds {
        match *f {
            FoldLine::Vertical { x, y0, y1 } => {
                out.push(dash((x, y1), (x, y1 + len))); // head tick
                out.push(dash((x, y0), (x, y0 - len))); // foot tick
            }
            FoldLine::Horizontal { y, x0, x1 } => {
                out.push(dash((x1, y), (x1 + len, y))); // right tick
                out.push(dash((x0, y), (x0 - len, y))); // left tick
            }
        }
    }
    out
}

/// Back-step collation marks: a filled tab on the spine, stepped down by signature number so the
/// gathered book shows a descending staircase (perfect-bind sequencing). One tab per surface.
fn collation_marks(
    cm: &CollationMarks,
    folds: &[FoldLine],
    content: Rect,
    signature: usize,
) -> Vec<MarkPrimitive> {
    // Anchor on the first vertical fold (the spine); fall back to the content centre.
    let spine_x = folds
        .iter()
        .find_map(|f| match f {
            FoldLine::Vertical { x, .. } => Some(*x),
            _ => None,
        })
        .unwrap_or_else(|| content.center().0);
    // Step down from the head by (signature-1) tab heights.
    let step = signature.saturating_sub(1) as f64 * cm.height_pt;
    let top = content.ury - step;
    let rect = Rect::new(
        spine_x - cm.width_pt / 2.0,
        top - cm.height_pt,
        spine_x + cm.width_pt / 2.0,
        top,
    );
    vec![MarkPrimitive::FillRect {
        rect,
        color: cm.color.clone(),
    }]
}

/// Colour control bar: a horizontal row of solid patches in a sheet-edge region.
fn color_bar(cb: &ColorBar, sheet: Rect, gripper: f64) -> Vec<MarkPrimitive> {
    let swatches: Vec<MarkColor> = match cb.kind {
        ColorBarKind::ProcessSolids => vec![
            MarkColor::Process {
                c: 1.0,
                m: 0.0,
                y: 0.0,
                k: 0.0,
            },
            MarkColor::Process {
                c: 0.0,
                m: 1.0,
                y: 0.0,
                k: 0.0,
            },
            MarkColor::Process {
                c: 0.0,
                m: 0.0,
                y: 1.0,
                k: 0.0,
            },
            MarkColor::Process {
                c: 0.0,
                m: 0.0,
                y: 0.0,
                k: 1.0,
            },
        ],
        ColorBarKind::TintRamp => [0.25, 0.5, 0.75, 1.0]
            .iter()
            .map(|&k| MarkColor::Process {
                c: 0.0,
                m: 0.0,
                y: 0.0,
                k,
            })
            .collect(),
        // Spot patches need per-job spot data we don't carry yet.
        ColorBarKind::SpotPatches => Vec::new(),
    };
    let p = cb.patch_pt;
    let (mut x, y) = region_origin(cb.region, sheet, gripper, swatches.len() as f64 * p, p);
    let mut out = Vec::new();
    for color in swatches {
        out.push(MarkPrimitive::FillRect {
            rect: Rect::new(x, y, x + p, y + p),
            color,
        });
        x += p;
    }
    out
}

/// Job barcode as filled Code-128 bars in a sheet-edge region. Non-Code128 symbologies and empty
/// payloads emit nothing (deferred).
fn barcode_marks(bc: &Barcode, sheet: Rect, gripper: f64) -> Vec<MarkPrimitive> {
    if !matches!(bc.symbology, Symbology::Code128) {
        return Vec::new();
    }
    let Some(elements) = barcode::code128b_elements(&bc.data) else {
        return Vec::new();
    };
    let (w, h) = (108.0, 18.0);
    let (x, y) = region_origin(bc.region, sheet, gripper, w, h);
    barcode::bars_in_rect(&elements, Rect::new(x, y, x + w, y + h))
        .into_iter()
        .map(|rect| MarkPrimitive::FillRect {
            rect,
            color: MarkColor::Process {
                c: 0.0,
                m: 0.0,
                y: 0.0,
                k: 1.0,
            },
        })
        .collect()
}

/// Lower-left origin for a `w × h` furniture block parked in a sheet-edge region.
fn region_origin(region: MarkRegion, sheet: Rect, gripper: f64, w: f64, h: f64) -> (f64, f64) {
    let m = FURNITURE_MARGIN;
    match region {
        MarkRegion::Slug | MarkRegion::Foot => (sheet.llx + m, sheet.lly + gripper + m),
        MarkRegion::Head => (sheet.llx + m, sheet.ury - m - h),
        MarkRegion::Left => (sheet.llx + m, sheet.lly + gripper + m),
        MarkRegion::Right => (sheet.urx - m - w, sheet.lly + gripper + m),
    }
}

/// Resolve slug tokens to a positioned text run. Resolves only fields with a concrete, deterministic
/// data source (filename, sheet number, surface, custom literal); time/operator/job/separation
/// fields are skipped (no wall-clock for determinism; no data source yet).
fn slug_text(slug: &Slug, sheet: Rect, gripper: f64, ctx: &MarkContext) -> Option<MarkText> {
    let parts: Vec<String> = slug
        .fields
        .iter()
        .filter_map(|f| match f {
            SlugField::FileName => Some(ctx.file_name.to_string()),
            SlugField::SheetNumber => Some(ctx.sheet_number.to_string()),
            SlugField::Surface => Some(match ctx.surface {
                SurfaceSide::Front => "front".to_string(),
                SurfaceSide::Back => "back".to_string(),
            }),
            SlugField::Custom(s) => Some(s.clone()),
            // Deferred / non-deterministic tokens.
            SlugField::DateTime
            | SlugField::Separation
            | SlugField::Operator
            | SlugField::JobNumber => None,
        })
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let text = parts.join(" \u{b7} "); // middot field delimiter (not authored copy)
    let size = slug.font_pt;
    let width = est_text_width(&text, size);
    let m = FURNITURE_MARGIN;
    let (x, y) = match slug.position {
        SlugPosition::BottomLeft => (sheet.llx + m, sheet.lly + gripper + m),
        SlugPosition::BottomCenter => (sheet.center().0 - width / 2.0, sheet.lly + gripper + m),
        SlugPosition::BottomRight => (sheet.urx - m - width, sheet.lly + gripper + m),
        SlugPosition::TopLeft => (sheet.llx + m, sheet.ury - m - size),
        SlugPosition::TopCenter => (sheet.center().0 - width / 2.0, sheet.ury - m - size),
        SlugPosition::TopRight => (sheet.urx - m - width, sheet.ury - m - size),
    };
    Some(MarkText {
        x,
        y,
        size,
        text,
        color: MarkColor::Process {
            c: 0.0,
            m: 0.0,
            y: 0.0,
            k: 1.0,
        },
    })
}

/// Approximate the set width of a Helvetica string (average advance ≈ 0.5 em). Deterministic;
/// used only to right/centre-align slug furniture, not for kerned layout.
fn est_text_width(text: &str, size: f64) -> f64 {
    text.chars().count() as f64 * size * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use vgdi_types::{CenterMarks, CropMarks, RegistrationMarks};

    fn frame() -> (Rect, Rect, Rect) {
        let trim = Rect::new(100.0, 100.0, 300.0, 400.0);
        let bleed = Rect::new(91.0, 91.0, 309.0, 409.0); // 9pt bleed
        let media = Rect::new(0.0, 0.0, 400.0, 500.0);
        (trim, bleed, media)
    }

    #[test]
    fn empty_markset_emits_nothing() {
        let (t, b, m) = frame();
        assert!(compute_marks(t, b, m, &MarkSet::default()).is_empty());
    }

    #[test]
    fn classic_crop_emits_eight_ticks_clamped_outside_bleed() {
        let (t, b, m) = frame();
        let mut ms = MarkSet::default();
        ms.crop = Some(CropMarks {
            offset_pt: 2.0,
            ..CropMarks::default()
        }); // 2 < 9pt bleed
        let prims = compute_marks(t, b, m, &ms);
        let lines: Vec<_> = prims
            .iter()
            .filter(|p| matches!(p, MarkPrimitive::Line { .. }))
            .collect();
        assert_eq!(lines.len(), 8, "classic crop = 8 ticks");
        let has_clamped = prims.iter().any(|p| {
            matches!(p,
            MarkPrimitive::Line { from, .. } if (from.0 - (100.0 - 9.0)).abs() < 1e-6)
        });
        assert!(has_clamped, "crop offset must clamp to the bleed amount");
    }

    #[test]
    fn full_line_crop_emits_four_lines() {
        let (t, b, m) = frame();
        let mut ms = MarkSet::default();
        ms.crop = Some(CropMarks {
            style: CropStyle::FullLine,
            ..CropMarks::default()
        });
        let n = compute_marks(t, b, m, &ms).len();
        assert_eq!(n, 4);
    }

    #[test]
    fn center_marks_four() {
        let (t, b, m) = frame();
        let mut ms = MarkSet::default();
        ms.center = Some(CenterMarks::default());
        assert_eq!(compute_marks(t, b, m, &ms).len(), 4);
    }

    #[test]
    fn registration_all_uses_registration_colorant() {
        let (t, b, m) = frame();
        let mut ms = MarkSet::default();
        ms.registration = Some(RegistrationMarks {
            positions: RegPositions::All,
            ..RegistrationMarks::default()
        });
        let prims = compute_marks(t, b, m, &ms);
        assert_eq!(prims.len(), 8, "edge-centers + corners");
        assert!(prims.iter().all(|p| matches!(
            p,
            MarkPrimitive::Circle {
                color: MarkColor::RegistrationAll,
                ..
            }
        )));
    }

    #[test]
    fn bleed_outline_emits_one_rect_at_bleed() {
        let (t, b, m) = frame();
        let mut ms = MarkSet::default();
        ms.bleed = BleedTreatment::Outline { weight_pt: 0.5 };
        let prims = compute_marks(t, b, m, &ms);
        assert_eq!(
            prims,
            vec![MarkPrimitive::Rect {
                rect: b,
                weight: 0.5,
                color: MarkColor::RegistrationAll
            }]
        );
    }

    #[test]
    fn crop_clamp_clears_widest_bleed_margin() {
        // Asymmetric bleed (left 2, bottom 1, right 40, top 20): crop ticks must clear the *widest*
        // margin, so the right-side ticks start at or beyond the bleed's right edge (SPEC §8 #6).
        let trim = Rect::new(100.0, 100.0, 300.0, 400.0);
        let bleed = Rect::new(98.0, 99.0, 340.0, 420.0);
        let media = Rect::new(0.0, 0.0, 500.0, 600.0);
        let mut ms = MarkSet::default();
        ms.crop = Some(CropMarks {
            offset_pt: 0.0,
            ..CropMarks::default()
        });
        let prims = compute_marks(trim, bleed, media, &ms);
        let right_horiz_start = prims
            .iter()
            .filter_map(|p| match p {
                MarkPrimitive::Line { from, to, .. }
                    if (from.1 - to.1).abs() < 1e-9 && from.0 > trim.urx + 1e-6 =>
                {
                    Some(from.0)
                }
                _ => None,
            })
            .fold(f64::INFINITY, f64::min);
        assert!(
            right_horiz_start >= bleed.urx - 1e-6,
            "right crop tick x={right_horiz_start} must clear bleed.urx={}",
            bleed.urx
        );
    }

    // ---- surface planner ----

    fn placed() -> Vec<PlacedCell> {
        // Two side-by-side pages (a booklet spread): left + right trims, each with a 9pt bleed.
        vec![
            PlacedCell {
                trim: Rect::new(50.0, 100.0, 250.0, 400.0),
                bleed: Rect::new(41.0, 91.0, 259.0, 409.0),
            },
            PlacedCell {
                trim: Rect::new(350.0, 100.0, 550.0, 400.0),
                bleed: Rect::new(341.0, 91.0, 559.0, 409.0),
            },
        ]
    }

    fn surface_input<'a>(
        cells: &'a [PlacedCell],
        folds: &'a [FoldLine],
        marks: &'a MarkSet,
        ctx: MarkContext<'a>,
    ) -> SurfaceMarkInput<'a> {
        SurfaceMarkInput {
            cells,
            fold_lines: folds,
            sheet: Rect::new(0.0, 0.0, 600.0, 500.0),
            gripper: 0.0,
            layout: SurfaceLayout::Independent,
            marks,
            ctx,
        }
    }

    fn ctx() -> MarkContext<'static> {
        MarkContext {
            file_name: "body.pdf",
            sheet_number: 1,
            surface: SurfaceSide::Front,
            signature: Some(1),
        }
    }

    #[test]
    fn surface_marks_are_per_cell_plus_one_registration_set() {
        let cells = placed();
        let mut ms = MarkSet::default();
        ms.crop = Some(CropMarks::default());
        ms.registration = Some(RegistrationMarks {
            positions: RegPositions::EdgeCenters,
            ..RegistrationMarks::default()
        });
        let plan = plan_surface_marks(&surface_input(&cells, &[], &ms, ctx()));
        let circles = plan
            .primitives
            .iter()
            .filter(|p| matches!(p, MarkPrimitive::Circle { .. }))
            .count();
        let ticks = plan
            .primitives
            .iter()
            .filter(|p| matches!(p, MarkPrimitive::Line { .. }))
            .count();
        assert_eq!(ticks, 16, "8 crop ticks per page × 2 pages");
        assert_eq!(circles, 4, "one edge-centre registration set for the sheet");
    }

    #[test]
    fn folded_spread_crops_outer_perimeter_only_no_spine_ticks() {
        // A booklet spread is one folded leaf: crop ticks frame the *whole spread* (8 ticks), never
        // the inner spine edges — cutting there would slice the fold.
        let cells = placed();
        let mut ms = MarkSet::default();
        ms.crop = Some(CropMarks::default());
        let mut input = surface_input(&cells, &[], &ms, ctx());
        input.layout = SurfaceLayout::Folded;
        let plan = plan_surface_marks(&input);
        let ticks: Vec<_> = plan
            .primitives
            .iter()
            .filter_map(|p| match p {
                MarkPrimitive::Line { from, .. } => Some(from.0),
                _ => None,
            })
            .collect();
        assert_eq!(ticks.len(), 8, "one outer frame, not per-page");
        // The spread's inner edges are at x=250 (left page right) and x=350 (right page left); no
        // crop tick should originate near the spine — only near the outer edges (x≈50 and x≈550).
        assert!(
            ticks.iter().all(|&x| x < 100.0 || x > 500.0),
            "no crop ticks at the spine, got xs {ticks:?}"
        );
    }

    #[test]
    fn folded_half_blank_spread_frames_full_leaf_no_spine_tick() {
        // A blank-pad spread places only the left page; with the spine fold provided, crop reflects
        // about the spine to frame the full symmetric leaf — no crop tick at the fold.
        let cells = vec![PlacedCell {
            trim: Rect::new(50.0, 100.0, 250.0, 400.0), // left page only
            bleed: Rect::new(50.0, 100.0, 250.0, 400.0),
        }];
        let folds = [FoldLine::Vertical {
            x: 300.0,
            y0: 100.0,
            y1: 400.0,
        }];
        let mut ms = MarkSet::default();
        ms.crop = Some(CropMarks::default());
        let mut input = surface_input(&cells, &folds, &ms, ctx());
        input.layout = SurfaceLayout::Folded;
        let plan = plan_surface_marks(&input);
        let xs: Vec<f64> = plan
            .primitives
            .iter()
            .filter_map(|p| match p {
                MarkPrimitive::Line { from, .. } => Some(from.0),
                _ => None,
            })
            .collect();
        assert_eq!(xs.len(), 8, "one outer frame for the full leaf");
        // Reflected leaf is [50 .. 550] about spine 300: ticks hug both outer edges, none at the fold.
        assert!(
            xs.iter().all(|&x| (x - 300.0).abs() > 40.0),
            "no spine tick, xs={xs:?}"
        );
        assert!(xs.iter().any(|&x| x < 100.0) && xs.iter().any(|&x| x > 500.0));
    }

    #[test]
    fn slug_resolves_tokens_into_a_text_run_in_the_slug_area() {
        let cells = placed();
        let mut ms = MarkSet::default();
        ms.slug = Some(Slug::default()); // filename · sheet · surface
        let plan = plan_surface_marks(&surface_input(&cells, &[], &ms, ctx()));
        assert_eq!(plan.texts.len(), 1);
        let t = &plan.texts[0];
        assert!(t.text.contains("body.pdf"));
        assert!(t.text.contains("front"));
        assert!(t.text.contains('1'));
        // Bottom-left slug sits near the lower-left corner.
        assert!(t.x < 50.0 && t.y < 50.0);
    }

    #[test]
    fn slug_skips_nondeterministic_fields() {
        let cells = placed();
        let mut ms = MarkSet::default();
        ms.slug = Some(Slug {
            fields: vec![SlugField::DateTime, SlugField::Operator],
            ..Slug::default()
        });
        let plan = plan_surface_marks(&surface_input(&cells, &[], &ms, ctx()));
        assert!(plan.texts.is_empty(), "all fields were deferred -> no run");
    }

    #[test]
    fn color_bar_emits_four_process_solids() {
        let cells = placed();
        let mut ms = MarkSet::default();
        ms.color_bar = Some(ColorBar::default());
        let plan = plan_surface_marks(&surface_input(&cells, &[], &ms, ctx()));
        let fills: Vec<_> = plan
            .primitives
            .iter()
            .filter(|p| matches!(p, MarkPrimitive::FillRect { .. }))
            .collect();
        assert_eq!(fills.len(), 4, "C M Y K solids");
    }

    #[test]
    fn fold_marks_emit_two_dashed_ticks_per_vertical_fold() {
        let cells = placed();
        let folds = [FoldLine::Vertical {
            x: 300.0,
            y0: 100.0,
            y1: 400.0,
        }];
        let mut ms = MarkSet::default();
        ms.fold = Some(FoldMarks::default());
        let plan = plan_surface_marks(&surface_input(&cells, &folds, &ms, ctx()));
        let dashed: Vec<_> = plan
            .primitives
            .iter()
            .filter(|p| matches!(p, MarkPrimitive::Line { dash, .. } if *dash > 0.0))
            .collect();
        assert_eq!(dashed.len(), 2, "head + foot fold ticks");
    }

    #[test]
    fn collation_steps_down_with_signature_number() {
        let cells = placed();
        let folds = [FoldLine::Vertical {
            x: 300.0,
            y0: 100.0,
            y1: 400.0,
        }];
        let mut ms = MarkSet::default();
        ms.collation = Some(CollationMarks::default());
        let tab = |sig| {
            let c = ctx();
            let ctx = MarkContext {
                signature: Some(sig),
                ..c
            };
            let plan = plan_surface_marks(&surface_input(&cells, &folds, &ms, ctx));
            match plan.primitives.iter().find_map(|p| match p {
                MarkPrimitive::FillRect { rect, .. } => Some(*rect),
                _ => None,
            }) {
                Some(r) => r,
                None => panic!("expected a collation tab"),
            }
        };
        let s1 = tab(1);
        let s2 = tab(2);
        assert!(s2.ury < s1.ury, "signature 2 steps below signature 1");
    }

    #[test]
    fn barcode_emits_bars_when_data_present() {
        let cells = placed();
        let mut ms = MarkSet::default();
        ms.job_barcode = Some(Barcode {
            data: "JOB-4471".into(),
            ..Barcode::default()
        });
        let plan = plan_surface_marks(&surface_input(&cells, &[], &ms, ctx()));
        let bars = plan
            .primitives
            .iter()
            .filter(|p| matches!(p, MarkPrimitive::FillRect { .. }))
            .count();
        assert!(bars > 10, "a Code-128 symbol has many bars, got {bars}");
    }

    #[test]
    fn gang_crop_marks_double_with_bleed_single_without() {
        // 3 cards in a row, 10pt gaps. With bleed each card's trim is a separate cut → double mark
        // at each interior boundary (6 distinct vertical cut lines); without bleed neighbours share
        // the cut → single (4 distinct lines).
        let row = |bleed_amt: f64| -> Vec<PlacedCell> {
            (0..3)
                .map(|i| {
                    let x0 = i as f64 * 110.0;
                    let trim = Rect::new(x0, 0.0, x0 + 100.0, 60.0);
                    let bleed = Rect::new(
                        trim.llx - bleed_amt,
                        trim.lly - bleed_amt,
                        trim.urx + bleed_amt,
                        trim.ury + bleed_amt,
                    );
                    PlacedCell { trim, bleed }
                })
                .collect()
        };
        let crop = CropMarks::default();
        let vert_cut_lines = |cells: &[PlacedCell]| {
            let mut xs: Vec<f64> = gang_crop_marks(cells, &crop)
                .iter()
                .filter_map(|p| match p {
                    MarkPrimitive::Line { from, to, .. } if (from.0 - to.0).abs() < 1e-9 => {
                        Some((from.0 * 1000.0).round())
                    }
                    _ => None,
                })
                .collect();
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            xs.dedup();
            xs.len()
        };
        assert_eq!(
            vert_cut_lines(&row(5.0)),
            6,
            "bleed → double interior marks"
        );
        assert_eq!(
            vert_cut_lines(&row(0.0)),
            4,
            "no bleed → single shared marks"
        );
    }
}
