//! Output press sheet + binding configuration.

use crate::Pt;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sheet {
    /// `[width, height]` in points.
    pub size_pt: [Pt; 2],
    /// Non-imageable gripper margin reserved on the gripper edge, in points.
    #[serde(default)]
    pub gripper_pt: Pt,
    /// Imageable-area inset reserved on all four edges, in points. The imposition grid is laid out
    /// inside `[margin .. size - margin]` (plus the gripper on the gripper edge), leaving a band for
    /// sheet-edge printer marks (crop/registration/colour bar/slug) so they aren't clipped or laid
    /// over the outermost pages. Defaults to 0 (fill the sheet).
    #[serde(default)]
    pub margin_pt: Pt,
    #[serde(default)]
    pub work_style: WorkStyle,
    /// Optional extra fold-over allowance for the topmost signature (hardcover wrap).
    #[serde(default)]
    pub flip: Option<Flip>,
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

/// "Flip": extra space added at the binding edge of the outermost/topmost signature so it can be
/// folded over a hardcover board. M1 models the allowance; rendering it is a deferred spec.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Flip {
    pub allowance_pt: Pt,
}
