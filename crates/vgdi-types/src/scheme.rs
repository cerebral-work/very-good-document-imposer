//! Imposition schemes (SPEC §10 / M1 design).

use crate::Pt;
use serde::{Deserialize, Serialize};

/// The imposition rule.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Scheme {
    NUp(NUp),
    StepRepeat(StepRepeat),
    SaddleStitch(SaddleStitch),
    PerfectBound(PerfectBound),
    Manual(Manual),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NUp {
    pub rows: u32,
    pub cols: u32,
    #[serde(default)]
    pub fill: FillOrder,
    #[serde(default)]
    pub scale: ScaleMode,
    #[serde(default)]
    pub gutter_pt: Pt,
    /// Rotate a page 90° if it then fits the cell better.
    #[serde(default)]
    pub rotate_to_fit: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepRepeat {
    pub rows: u32,
    pub cols: u32,
    /// Horizontal / vertical space between repeats, in points.
    #[serde(default)]
    pub h_space_pt: Pt,
    #[serde(default)]
    pub v_space_pt: Pt,
    #[serde(default)]
    pub bleed_mode: BleedMode,
    #[serde(default)]
    pub scale: ScaleMode,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SaddleStitch {
    #[serde(default)]
    pub duplex: Duplex,
    /// Treat the first/last pages as a separate cover signature.
    #[serde(default)]
    pub cover: bool,
    #[serde(default)]
    pub scale: ScaleMode,
    /// Spine gutter between the two pages of a spread, in points.
    #[serde(default)]
    pub spine_pt: Pt,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PerfectBound {
    /// Pages per gathered signature (rounded up to a multiple of 4).
    pub signature_pages: u32,
    #[serde(default)]
    pub duplex: Duplex,
    #[serde(default)]
    pub scale: ScaleMode,
    #[serde(default)]
    pub spine_pt: Pt,
}

/// Fully manual placement: explicit cells per surface (custom layouts, autocutter work).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manual {
    pub surfaces: Vec<ManualSurface>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManualSurface {
    #[serde(default)]
    pub side: SurfaceSide,
    pub placements: Vec<ManualPlacement>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManualPlacement {
    /// Source id (defaults to the first source).
    #[serde(default)]
    pub source: Option<String>,
    pub page: usize,
    /// Lower-left anchor of the placed trim box, in sheet points.
    pub x_pt: Pt,
    pub y_pt: Pt,
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default)]
    pub rotate: i32,
    #[serde(default)]
    pub mirror: bool,
}

fn one() -> f64 {
    1.0
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FillOrder {
    #[default]
    RowMajor,
    ColMajor,
}

/// Per-cell scale mode. `Fit` = uniform fit to cell; `Fixed` = explicit factor.
/// Anamorphic (non-uniform) scaling is v1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScaleMode {
    None,
    #[default]
    Fit,
    Fixed(f64),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BleedMode {
    #[default]
    Bleed,
    NoBleed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Duplex {
    #[default]
    LongEdge,
    ShortEdge,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SurfaceSide {
    #[default]
    Front,
    Back,
}
