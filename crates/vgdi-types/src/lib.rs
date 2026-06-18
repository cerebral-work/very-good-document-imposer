//! Domain model for Very Good Document Imposer.
//!
//! Serializable contract the CLI consumes and the (future) GUI edits. Pure: no PDF/IO deps.
//! Geometry is in PDF points (1 pt = 1/72 inch), origin bottom-left, per ISO 32000.

use serde::{Deserialize, Serialize};

pub mod job;
pub mod marks;
pub mod scheme;
pub mod sheet;

pub use job::*;
pub use marks::*;
pub use scheme::*;
pub use sheet::*;

/// PDF points (1/72 inch).
pub type Pt = f64;

/// The current JobSpec schema identifier (additive changes stay on @1).
pub const SCHEMA_ID: &str = "vgdi/jobspec@1";

/// An axis-aligned rectangle in PDF user space `[llx lly urx ury]`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub llx: Pt,
    pub lly: Pt,
    pub urx: Pt,
    pub ury: Pt,
}

impl Rect {
    pub fn new(llx: Pt, lly: Pt, urx: Pt, ury: Pt) -> Self {
        Rect {
            llx: llx.min(urx),
            lly: lly.min(ury),
            urx: llx.max(urx),
            ury: lly.max(ury),
        }
    }
    pub fn width(&self) -> Pt {
        self.urx - self.llx
    }
    pub fn height(&self) -> Pt {
        self.ury - self.lly
    }
    pub fn center(&self) -> (Pt, Pt) {
        ((self.llx + self.urx) / 2.0, (self.lly + self.ury) / 2.0)
    }
    /// Shrink (positive `d`) or grow (negative `d`) on all sides.
    pub fn inset(&self, d: Pt) -> Rect {
        Rect::new(self.llx + d, self.lly + d, self.urx - d, self.ury - d)
    }
    /// True if `inner` is contained within `self`, allowing a small float epsilon of slack.
    pub fn contains(&self, inner: &Rect, eps: Pt) -> bool {
        inner.llx >= self.llx - eps
            && inner.lly >= self.lly - eps
            && inner.urx <= self.urx + eps
            && inner.ury <= self.ury + eps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_normalizes_and_contains() {
        let outer = Rect::new(0.0, 0.0, 100.0, 100.0);
        let inner = Rect::new(10.0, 10.0, 90.0, 90.0);
        assert!(outer.contains(&inner, 1e-6));
        assert!(!inner.contains(&outer, 1e-6));
        let r = Rect::new(90.0, 90.0, 10.0, 10.0);
        assert_eq!(r.width(), 80.0);
        assert_eq!(r.llx, 10.0);
    }

    #[test]
    fn rect_center_and_inset() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        assert_eq!(r.center(), (50.0, 25.0));
        let i = r.inset(10.0);
        assert_eq!((i.llx, i.lly, i.urx, i.ury), (10.0, 10.0, 90.0, 40.0));
    }
}
