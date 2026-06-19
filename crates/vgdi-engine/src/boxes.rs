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

/// Largest per-side margin (points) treated as *natural* bleed when no BleedBox is declared. ~12.7mm
/// — generous for real bleeds (typically 3 mm), tight enough that a small trim on an oversized
/// artboard isn't read as a giant bleed. (Tunable; a future preference can override.)
const MAX_NATURAL_BLEED_PT: f64 = 36.0;

impl PageBoxes {
    /// The authoritative placement reference: TrimBox, else ArtBox, else `None` (reject).
    pub fn effective_trim(&self) -> Option<Rect> {
        self.trim.or(self.art)
    }

    /// The bleed extent used for bleed-pull. An explicit BleedBox is honoured as-is. Otherwise the
    /// page's **natural bleed** is recognised — content carried past the trim out to the CropBox,
    /// else the MediaBox — but **capped** to [`MAX_NATURAL_BLEED_PT`] per side, so an oversized
    /// artboard (a small trim centred on a huge MediaBox, no BleedBox) is not mistaken for a giant
    /// bleed. This is the default (QI recognises embedded bleed too); a future preference can force
    /// strict BleedBox-or-trim behaviour.
    pub fn effective_bleed(&self) -> Option<Rect> {
        if let Some(b) = self.bleed {
            return Some(b);
        }
        let trim = self.effective_trim().unwrap_or(self.media);
        let natural = self.crop.unwrap_or(self.media);
        Some(Rect::new(
            natural.llx.max(trim.llx - MAX_NATURAL_BLEED_PT),
            natural.lly.max(trim.lly - MAX_NATURAL_BLEED_PT),
            natural.urx.min(trim.urx + MAX_NATURAL_BLEED_PT),
            natural.ury.min(trim.ury + MAX_NATURAL_BLEED_PT),
        ))
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
    fn effective_bleed_recognises_natural_bleed_from_media() {
        // No explicit BleedBox/CropBox, but the MediaBox has a 3mm margin around the trim — that
        // margin is the natural bleed (like the testcard / QI default).
        let b = PageBoxes {
            media: rect(0.0, 0.0, 100.0, 100.0),
            crop: None,
            trim: Some(rect(8.5, 8.5, 91.5, 91.5)),
            art: None,
            bleed: None,
        };
        assert_eq!(b.effective_bleed().unwrap(), rect(0.0, 0.0, 100.0, 100.0));

        // An explicit BleedBox still wins.
        let b2 = PageBoxes {
            bleed: Some(rect(5.0, 5.0, 95.0, 95.0)),
            ..b
        };
        assert_eq!(b2.effective_bleed().unwrap(), rect(5.0, 5.0, 95.0, 95.0));

        // A CropBox is preferred over the MediaBox (don't pull hidden content).
        let b3 = PageBoxes {
            bleed: None,
            crop: Some(rect(3.0, 3.0, 97.0, 97.0)),
            ..b
        };
        assert_eq!(b3.effective_bleed().unwrap(), rect(3.0, 3.0, 97.0, 97.0));
    }

    #[test]
    fn natural_bleed_is_capped_for_oversized_artboards() {
        // A small trim centred on a huge MediaBox (no BleedBox) is an artboard, not a 450pt bleed:
        // the inferred bleed is capped to 36pt per side around the trim.
        let b = PageBoxes {
            media: rect(0.0, 0.0, 1000.0, 1000.0),
            crop: None,
            trim: Some(rect(450.0, 450.0, 550.0, 550.0)),
            art: None,
            bleed: None,
        };
        assert_eq!(
            b.effective_bleed().unwrap(),
            rect(414.0, 414.0, 586.0, 586.0),
            "bleed capped to trim ± 36pt"
        );
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
