//! Domain model for Very Good Document Imposer.
//!
//! This crate is the serializable contract the CLI consumes and the (future) GUI edits.
//! It is pure: no PDF or I/O dependencies. Geometry is in PDF points (1 pt = 1/72 inch),
//! origin bottom-left, per ISO 32000.
//!
//! M0 scope: `Scheme::NUp` only. Other variants are declared for forward-compatibility but
//! are rejected by the M0 planner.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// PDF points (1/72 inch).
pub type Pt = f64;

/// The current JobSpec schema identifier. Bumped on breaking schema changes.
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
        // Normalize so ll is the minimum corner regardless of how the source ordered them.
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
    /// True if `inner` is contained within `self`, allowing a small float epsilon of slack.
    pub fn contains(&self, inner: &Rect, eps: Pt) -> bool {
        inner.llx >= self.llx - eps
            && inner.lly >= self.lly - eps
            && inner.urx <= self.urx + eps
            && inner.ury <= self.ury + eps
    }
}

/// Root job specification: the single source of truth (SPEC §7).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSpec {
    /// Must equal [`SCHEMA_ID`].
    pub schema: String,
    pub sources: Vec<SourceRef>,
    pub scheme: Scheme,
    pub sheet: Sheet,
    #[serde(default)]
    pub color_policy: ColorPolicy,
    #[serde(default)]
    pub output: OutputTarget,
}

/// A referenced input PDF.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRef {
    pub id: String,
    pub path: PathBuf,
}

/// The imposition rule. M0 supports only `NUp`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Scheme {
    NUp(NUp),
    /// Declared for forward-compatibility; rejected by the M0 planner.
    StepRepeat(StepRepeat),
    /// Declared for forward-compatibility; rejected by the M0 planner.
    Booklet(Booklet),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NUp {
    pub rows: u32,
    pub cols: u32,
    #[serde(default)]
    pub fill: FillOrder,
    #[serde(default)]
    pub scale: ScaleMode,
    /// Gutter between cells, in points.
    #[serde(default)]
    pub gutter_pt: Pt,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepRepeat {
    pub rows: u32,
    pub cols: u32,
    #[serde(default)]
    pub gutter_pt: Pt,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Booklet {
    #[serde(default)]
    pub duplex: Duplex,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Duplex {
    #[default]
    LongEdge,
    ShortEdge,
}

/// Cell fill order for grid schemes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FillOrder {
    #[default]
    RowMajor,
    ColMajor,
}

/// Per-cell scale mode. M0 supports `None` (1:1) and `Fit` (uniform fit to cell).
/// Anamorphic (non-uniform) scaling is deferred to v1 (SPEC §16).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScaleMode {
    None,
    #[default]
    Fit,
}

/// Output press sheet.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sheet {
    /// `[width, height]` in points.
    pub size_pt: [Pt; 2],
    /// Non-imageable gripper margin reserved on the gripper edge, in points.
    #[serde(default)]
    pub gripper_pt: Pt,
    #[serde(default)]
    pub work_style: WorkStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkStyle {
    #[default]
    Sheetwise,
    Perfector,
    WorkAndTurn,
    WorkAndTumble,
}

/// Color preservation policy (SPEC §8 invariant #5). Defaults preserve everything.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ColorPolicy {
    #[serde(default = "default_true")]
    pub preserve_spots: bool,
    #[serde(default = "default_true")]
    pub preserve_overprint: bool,
}

impl Default for ColorPolicy {
    fn default() -> Self {
        ColorPolicy {
            preserve_spots: true,
            preserve_overprint: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Output target configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OutputTarget {
    /// PDF/X conformance hint, e.g. `"X-4"`. M0 emits a generic PDF; conformance lands in v1.
    #[serde(default)]
    pub pdfx: Option<String>,
    /// Forced PDF version string, e.g. `"1.6"`.
    #[serde(default)]
    pub pdf_version: Option<String>,
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
        // swapped corners normalize
        let r = Rect::new(90.0, 90.0, 10.0, 10.0);
        assert_eq!(r.width(), 80.0);
        assert_eq!(r.llx, 10.0);
    }

    #[test]
    fn jobspec_roundtrips_via_serde_json() {
        let job = JobSpec {
            schema: SCHEMA_ID.to_string(),
            sources: vec![SourceRef {
                id: "body".into(),
                path: "body.pdf".into(),
            }],
            scheme: Scheme::NUp(NUp {
                rows: 2,
                cols: 2,
                fill: FillOrder::RowMajor,
                scale: ScaleMode::Fit,
                gutter_pt: 0.0,
            }),
            sheet: Sheet {
                size_pt: [595.276, 841.89],
                gripper_pt: 0.0,
                work_style: WorkStyle::Sheetwise,
            },
            color_policy: ColorPolicy::default(),
            output: OutputTarget::default(),
        };
        let s = serde_json::to_string(&job).unwrap();
        let back: JobSpec = serde_json::from_str(&s).unwrap();
        assert_eq!(back.schema, SCHEMA_ID);
        assert!(matches!(back.scheme, Scheme::NUp(_)));
    }
}
