//! PDF page-box model and the prepress-correctness rules around it (SPEC §8).
//!
//! Placement anchors on TrimBox, falling back to ArtBox; if both are absent the page is
//! non-conformant for pro prepress and is rejected — the engine never silently anchors on
//! CropBox/MediaBox (that would shift every page).

use vgdi_types::Rect;

/// The five PDF page boxes for one page. `media` is always present; the rest are optional.
#[derive(Clone, Copy, Debug)]
pub struct PageBoxes {
    pub media: Rect,
    pub crop: Option<Rect>,
    pub trim: Option<Rect>,
    pub art: Option<Rect>,
    pub bleed: Option<Rect>,
}

/// Float slack (in points) for containment checks. ~1/7000 inch — well below device resolution.
const EPS: f64 = 0.01;

impl PageBoxes {
    /// The authoritative placement reference: TrimBox, else ArtBox, else `None` (reject).
    pub fn effective_trim(&self) -> Option<Rect> {
        self.trim.or(self.art)
    }

    /// The bleed extent used for bleed-pull: explicit BleedBox, else the trim (no bleed).
    pub fn effective_bleed(&self) -> Option<Rect> {
        self.bleed.or_else(|| self.effective_trim())
    }

    /// Enforce `Media ⊇ Bleed ⊇ Trim/Art`. Returns `false` on violation.
    pub fn containment_ok(&self) -> bool {
        let Some(trim) = self.effective_trim() else {
            return false;
        };
        let bleed = self.effective_bleed().unwrap_or(trim);
        self.media.contains(&bleed, EPS) && bleed.contains(&trim, EPS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(llx: f64, lly: f64, urx: f64, ury: f64) -> Rect {
        Rect::new(llx, lly, urx, ury)
    }

    #[test]
    fn trim_preferred_then_art() {
        let b = PageBoxes {
            media: rect(0.0, 0.0, 100.0, 100.0),
            crop: None,
            trim: Some(rect(5.0, 5.0, 95.0, 95.0)),
            art: Some(rect(10.0, 10.0, 90.0, 90.0)),
            bleed: None,
        };
        assert_eq!(b.effective_trim().unwrap(), rect(5.0, 5.0, 95.0, 95.0));

        let b2 = PageBoxes { trim: None, ..b };
        assert_eq!(b2.effective_trim().unwrap(), rect(10.0, 10.0, 90.0, 90.0));
    }

    #[test]
    fn reject_when_no_trim_or_art() {
        let b = PageBoxes {
            media: rect(0.0, 0.0, 100.0, 100.0),
            crop: Some(rect(1.0, 1.0, 99.0, 99.0)),
            trim: None,
            art: None,
            bleed: None,
        };
        assert!(b.effective_trim().is_none());
        assert!(!b.containment_ok());
    }

    #[test]
    fn containment_violation_when_trim_exceeds_media() {
        let b = PageBoxes {
            media: rect(0.0, 0.0, 100.0, 100.0),
            crop: None,
            trim: Some(rect(-5.0, 0.0, 105.0, 100.0)), // wider than media
            art: None,
            bleed: None,
        };
        assert!(!b.containment_ok());
    }

    #[test]
    fn valid_nested_boxes_pass() {
        let b = PageBoxes {
            media: rect(0.0, 0.0, 100.0, 100.0),
            crop: None,
            trim: Some(rect(10.0, 10.0, 90.0, 90.0)),
            art: None,
            bleed: Some(rect(5.0, 5.0, 95.0, 95.0)),
        };
        assert!(b.containment_ok());
    }
}
