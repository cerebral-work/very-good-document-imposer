//! 2-D affine geometry in the PDF convention.
//!
//! A PDF matrix `[a b c d e f]` maps a point `(x, y)` to:
//!   `x' = a*x + c*y + e`
//!   `y' = b*x + d*y + f`
//!
//! This module is pure and deterministic; it is the geometry kernel the planner relies on,
//! so it is unit-tested directly (SPEC §13 placement golden).

use vgdi_types::{Rect, ScaleMode};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn translate(tx: f64, ty: f64) -> Matrix {
        Matrix {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    pub fn scale(sx: f64, sy: f64) -> Matrix {
        Matrix {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Rotation by a multiple of 90 degrees, clockwise (matching the PDF `/Rotate` convention:
    /// "degrees by which the page shall be rotated clockwise when displayed").
    pub fn rotate_cw(deg: i32) -> Matrix {
        match deg.rem_euclid(360) {
            0 => Matrix::IDENTITY,
            // (x,y) -> (y, -x)
            90 => Matrix {
                a: 0.0,
                b: -1.0,
                c: 1.0,
                d: 0.0,
                e: 0.0,
                f: 0.0,
            },
            // (x,y) -> (-x, -y)
            180 => Matrix {
                a: -1.0,
                b: 0.0,
                c: 0.0,
                d: -1.0,
                e: 0.0,
                f: 0.0,
            },
            // (x,y) -> (-y, x)
            270 => Matrix {
                a: 0.0,
                b: 1.0,
                c: -1.0,
                d: 0.0,
                e: 0.0,
                f: 0.0,
            },
            other => panic!("rotate_cw only supports multiples of 90, got {other}"),
        }
    }

    /// Inverse of this affine. Every CTM the planner builds is invertible (positive uniform scale +
    /// 90°-multiple rotation/mirror + translation), so `det != 0`; debug-asserts otherwise.
    pub fn inverse(&self) -> Matrix {
        let det = self.a * self.d - self.c * self.b;
        debug_assert!(det.abs() > f64::EPSILON, "non-invertible matrix");
        let inv = 1.0 / det;
        Matrix {
            a: self.d * inv,
            b: -self.b * inv,
            c: -self.c * inv,
            d: self.a * inv,
            e: (self.c * self.f - self.d * self.e) * inv,
            f: (self.b * self.e - self.a * self.f) * inv,
        }
    }

    /// Apply this matrix to a point.
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// Compose: returns a matrix equivalent to applying `inner` first, then `self`.
    /// (i.e. `self ∘ inner`; `result.apply(p) == self.apply(inner.apply(p))`).
    pub fn compose(&self, inner: &Matrix) -> Matrix {
        // self = [a b c d e f] (call S), inner = I.  result = S * I in PDF row-vector terms.
        let s = self;
        let i = inner;
        Matrix {
            a: i.a * s.a + i.b * s.c,
            b: i.a * s.b + i.b * s.d,
            c: i.c * s.a + i.d * s.c,
            d: i.c * s.b + i.d * s.d,
            e: i.e * s.a + i.f * s.c + s.e,
            f: i.e * s.b + i.f * s.d + s.f,
        }
    }

    /// Format as a PDF content-stream `cm` operand list, with stable rounding for determinism.
    pub fn to_pdf(&self) -> String {
        format!(
            "{} {} {} {} {} {}",
            fmt(self.a),
            fmt(self.b),
            fmt(self.c),
            fmt(self.d),
            fmt(self.e),
            fmt(self.f)
        )
    }
}

/// Deterministic number formatting: fixed 6 decimals, trailing zeros trimmed, `-0` normalized.
pub fn fmt(v: f64) -> String {
    let mut s = format!("{:.6}", v);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".to_string();
    }
    s
}

/// Axis-aligned bounds of an arbitrary rectangle after applying matrix `m`. Because every CTM the
/// planner produces is a multiple-of-90 rotation (optionally mirrored) plus uniform scale and
/// translation, the transformed rectangle stays axis-aligned, so its bounds are exact — this is how
/// a source trim/bleed box is mapped into sheet space for mark placement.
pub fn transform_rect_bounds(m: &Matrix, r: Rect) -> Rect {
    let mut minx = f64::INFINITY;
    let mut miny = f64::INFINITY;
    let mut maxx = f64::NEG_INFINITY;
    let mut maxy = f64::NEG_INFINITY;
    for (x, y) in [
        (r.llx, r.lly),
        (r.urx, r.lly),
        (r.llx, r.ury),
        (r.urx, r.ury),
    ] {
        let (rx, ry) = m.apply(x, y);
        minx = minx.min(rx);
        miny = miny.min(ry);
        maxx = maxx.max(rx);
        maxy = maxy.max(ry);
    }
    Rect::new(minx, miny, maxx, maxy)
}

/// Reflect an axis-aligned rect horizontally about the vertical line `x = axis` (sheet space). This
/// is the work-and-turn **position** transform `T`: it relocates the rect (mirrors its x range) while
/// the caller re-places page content *upright* into the result — content is never mirrored (SPEC §9,
/// "never apply a negative-x scale to page content"). Involutive: `reflect_x(reflect_x(r, a), a) == r`.
pub fn reflect_x(r: Rect, axis: f64) -> Rect {
    Rect::new(2.0 * axis - r.urx, r.lly, 2.0 * axis - r.llx, r.ury)
}

/// Axis-aligned bounds of the rectangle `[0,w] x [0,h]` after applying matrix `m`.
fn corners_bounds(w: f64, h: f64, m: &Matrix) -> (f64, f64, f64, f64) {
    let mut minx = f64::INFINITY;
    let mut miny = f64::INFINITY;
    let mut maxx = f64::NEG_INFINITY;
    let mut maxy = f64::NEG_INFINITY;
    for (x, y) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
        let (rx, ry) = m.apply(x, y);
        minx = minx.min(rx);
        miny = miny.min(ry);
        maxx = maxx.max(rx);
        maxy = maxy.max(ry);
    }
    (minx, miny, maxx, maxy)
}

/// The result of normalizing a placement: the CTM to paint the source page's content with,
/// the form `/BBox` (the source trim box, clipping in form space), and the visible size.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub ctm: Matrix,
    pub bbox: Rect,
    pub visible_w: f64,
    pub visible_h: f64,
}

/// Compute the placement matrix that maps a source page's TrimBox into `cell`, honoring the
/// page `/Rotate`, anchoring on the trim box (never Media/Crop), and centering within the cell.
///
/// The form XObject is painted as `q <ctm> cm /Xn Do Q` with `/Matrix [1 0 0 1 0 0]` and
/// `/BBox = trim` (clip in page space). All rotation/scale/translation lives in the CTM, so
/// the engine "owns" the geometry regardless of the PDF backend (SPEC §13).
pub fn place_trim_in_cell(trim: Rect, rotate: i32, cell: Rect, scale: ScaleMode) -> Placement {
    let rot = rotate.rem_euclid(360);
    let (w, h) = (trim.width(), trim.height());
    // Visible (post-rotation) dimensions.
    let (vw, vh) = if rot == 90 || rot == 270 {
        (h, w)
    } else {
        (w, h)
    };

    let s = match scale {
        ScaleMode::None => 1.0,
        ScaleMode::Fit => (cell.width() / vw).min(cell.height() / vh),
        ScaleMode::Fixed(f) => f,
    };

    let placed_w = vw * s;
    let placed_h = vh * s;
    let ox = cell.llx + (cell.width() - placed_w) / 2.0;
    let oy = cell.lly + (cell.height() - placed_h) / 2.0;

    // Build CTM = T(ox,oy) ∘ S(s) ∘ T(-min) ∘ R ∘ T(-trim.ll)
    //  - move trim lower-left to origin
    //  - rotate clockwise by /Rotate
    //  - shift rotated box back into the positive quadrant
    //  - scale uniformly
    //  - translate to the cell anchor
    let to_origin = Matrix::translate(-trim.llx, -trim.lly);
    let r = Matrix::rotate_cw(rot);
    // Shift the rotated box back into the positive quadrant.
    let (minx, miny, _, _) = corners_bounds(w, h, &r);
    let shift = Matrix::translate(-minx, -miny);
    let scale_m = Matrix::scale(s, s);
    let to_cell = Matrix::translate(ox, oy);

    let ctm = to_cell
        .compose(&scale_m)
        .compose(&shift)
        .compose(&r)
        .compose(&to_origin);

    Placement {
        ctm,
        bbox: trim,
        visible_w: placed_w,
        visible_h: placed_h,
    }
}

/// Like [`place_trim_in_cell`] but adds duplex back-side flip and optional rotate-to-fit.
/// `flip180` adds 180° (short-edge duplex back side); `rotate_to_fit` (only with `Fit`) tries the
/// page at +90° too and keeps whichever fills the cell more.
pub fn place_best(
    trim: Rect,
    page_rotate: i32,
    cell: Rect,
    scale: ScaleMode,
    rotate_to_fit: bool,
    flip180: bool,
) -> Placement {
    let base = page_rotate + if flip180 { 180 } else { 0 };
    if rotate_to_fit && matches!(scale, ScaleMode::Fit) {
        let a = place_trim_in_cell(trim, base, cell, scale);
        let b = place_trim_in_cell(trim, base + 90, cell, scale);
        if b.visible_w * b.visible_h > a.visible_w * a.visible_h {
            return b;
        }
        return a;
    }
    place_trim_in_cell(trim, base, cell, scale)
}

/// Manual placement: anchor the trim box's lower-left at `(x, y)` in sheet space, apply an
/// explicit `factor`, clockwise `rotate`, and optional horizontal `mirror`.
pub fn place_manual(
    trim: Rect,
    x: f64,
    y: f64,
    factor: f64,
    rotate: i32,
    mirror: bool,
) -> Placement {
    let (w, h) = (trim.width(), trim.height());
    let to_origin = Matrix::translate(-trim.llx, -trim.lly);
    let r = Matrix::rotate_cw(rotate);
    let m = if mirror {
        Matrix::scale(-1.0, 1.0)
    } else {
        Matrix::IDENTITY
    };
    let rm = m.compose(&r); // rotate first, then mirror
    let (minx, miny, maxx, maxy) = corners_bounds(w, h, &rm);
    let shift = Matrix::translate(-minx, -miny);
    let scale_m = Matrix::scale(factor, factor);
    let to_xy = Matrix::translate(x, y);
    let ctm = to_xy
        .compose(&scale_m)
        .compose(&shift)
        .compose(&rm)
        .compose(&to_origin);
    Placement {
        ctm,
        bbox: trim,
        visible_w: (maxx - minx) * factor,
        visible_h: (maxy - miny) * factor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "expected {a} ≈ {b}");
    }

    #[test]
    fn compose_matches_sequential_apply() {
        let m1 = Matrix::translate(10.0, 20.0);
        let m2 = Matrix::scale(2.0, 3.0);
        let composed = m1.compose(&m2); // apply m2 then m1
        let (x, y) = composed.apply(5.0, 7.0);
        let (sx, sy) = m2.apply(5.0, 7.0);
        let (ex, ey) = m1.apply(sx, sy);
        approx(x, ex);
        approx(y, ey);
        // numerically: scale (10,21) then translate -> (20,41)
        approx(x, 20.0);
        approx(y, 41.0);
    }

    #[test]
    fn unrotated_trim_anchors_at_cell_with_scale_none() {
        // non-zero-origin trim box
        let trim = Rect::new(10.0, 10.0, 110.0, 60.0); // 100 x 50
        let cell = Rect::new(200.0, 300.0, 400.0, 500.0); // 200 x 200
        let p = place_trim_in_cell(trim, 0, cell, ScaleMode::None);
        approx(p.visible_w, 100.0);
        approx(p.visible_h, 50.0);
        // centered: ox = 200 + (200-100)/2 = 250 ; oy = 300 + (200-50)/2 = 375
        let (llx, lly) = p.ctm.apply(trim.llx, trim.lly);
        let (urx, ury) = p.ctm.apply(trim.urx, trim.ury);
        approx(llx, 250.0);
        approx(lly, 375.0);
        approx(urx, 350.0);
        approx(ury, 425.0);
    }

    #[test]
    fn rotate90_nonzero_origin_trim_maps_into_cell_upright() {
        // The classic off-by-a-translation case (SPEC §13 placement golden).
        let trim = Rect::new(10.0, 10.0, 110.0, 60.0); // page-space 100 x 50
        let cell = Rect::new(0.0, 0.0, 300.0, 300.0);
        let p = place_trim_in_cell(trim, 90, cell, ScaleMode::None);
        // After 90° rotation visible size is swapped: 50 x 100.
        approx(p.visible_w, 50.0);
        approx(p.visible_h, 100.0);
        // The four trim corners must land within the cell, axis-aligned, forming a 50x100 box.
        let pts = [
            p.ctm.apply(trim.llx, trim.lly),
            p.ctm.apply(trim.urx, trim.lly),
            p.ctm.apply(trim.urx, trim.ury),
            p.ctm.apply(trim.llx, trim.ury),
        ];
        let (mut minx, mut miny, mut maxx, mut maxy) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for (x, y) in pts {
            assert!(x >= -1e-6 && x <= 300.0 + 1e-6, "x {x} outside cell");
            assert!(y >= -1e-6 && y <= 300.0 + 1e-6, "y {y} outside cell");
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        }
        approx(maxx - minx, 50.0);
        approx(maxy - miny, 100.0);
        // centered in 300x300: min corner at (125, 100)
        approx(minx, 125.0);
        approx(miny, 100.0);
    }

    #[test]
    fn inverse_round_trips_a_rotated_scaled_translate() {
        let m = Matrix::translate(10.0, 20.0)
            .compose(&Matrix::scale(2.0, 2.0))
            .compose(&Matrix::rotate_cw(90));
        let inv = m.inverse();
        let (x, y) = m.apply(3.0, 7.0);
        let (rx, ry) = inv.apply(x, y);
        approx(rx, 3.0);
        approx(ry, 7.0);
        // m ∘ m⁻¹ = identity
        let id = m.compose(&inv);
        approx(id.a, 1.0);
        approx(id.d, 1.0);
        approx(id.b, 0.0);
        approx(id.c, 0.0);
        approx(id.e, 0.0);
        approx(id.f, 0.0);
    }

    #[test]
    fn reflect_x_mirrors_x_keeps_y_and_is_involutive() {
        let r = Rect::new(10.0, 20.0, 40.0, 60.0);
        let axis = 100.0;
        let m = reflect_x(r, axis);
        // x mirrored about 100: [10,40] -> [160,190]; y untouched.
        approx(m.llx, 160.0);
        approx(m.urx, 190.0);
        approx(m.lly, 20.0);
        approx(m.ury, 60.0);
        // Reflecting twice returns the original rect.
        let back = reflect_x(m, axis);
        approx(back.llx, r.llx);
        approx(back.lly, r.lly);
        approx(back.urx, r.urx);
        approx(back.ury, r.ury);
    }

    #[test]
    fn fmt_is_deterministic_and_trims() {
        assert_eq!(fmt(1.0), "1");
        assert_eq!(fmt(1.500000), "1.5");
        assert_eq!(fmt(-0.0), "0");
        assert_eq!(fmt(0.123456789), "0.123457");
    }
}
