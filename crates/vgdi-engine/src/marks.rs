//! Printer-mark **geometry** computation (pure). Given a trim/bleed/media frame and a `MarkSet`,
//! produce vector primitives positioned per the prepress rules (SPEC §8.6). Turning these
//! primitives into a PDF content stream (incl. the Separation `All` colorant) is a deferred
//! milestone — see the `#[ignore]`d emission specs.

use vgdi_types::{BleedTreatment, CropStyle, MarkColor, MarkSet, Rect, RegPositions};

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
}

/// Compute mark primitives around a single trim frame.
/// `trim` ⊆ `bleed` ⊆ `media` are the finished, bleed, and media rectangles in sheet space.
pub fn compute_marks(trim: Rect, bleed: Rect, media: Rect, marks: &MarkSet) -> Vec<MarkPrimitive> {
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
        // Crop marks must sit outside the bleed: clamp the offset to at least the bleed amount.
        let bleed_amt = (trim.llx - bleed.llx).max(trim.lly - bleed.lly).max(0.0);
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
        // offset must have been clamped up to the 9pt bleed amount, so the BL horizontal tick
        // starts at trim.llx - 9, not trim.llx - 2.
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
}
