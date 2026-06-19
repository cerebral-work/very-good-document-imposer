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
    /// Whether to pull each page's bleed band into the cell (form `/BBox` = BleedBox). Defaults
    /// to `NoBleed`: N-up proofs usually place finished trims. Bleed-pull requires the trim gutter
    /// to be ≥ 2× the bleed amount so neighbouring bleeds don't overlap (SPEC §8.7).
    #[serde(default = "default_no_bleed")]
    pub bleed_mode: BleedMode,
}

fn default_no_bleed() -> BleedMode {
    BleedMode::NoBleed
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepRepeat {
    /// Maximum rows to place; **0 = fit as many as the sheet allows** (QI "Max rows").
    #[serde(default)]
    pub max_rows: u32,
    /// Maximum columns to place; **0 = fit as many as the sheet allows** (QI "Max cols").
    #[serde(default)]
    pub max_cols: u32,
    /// Horizontal / vertical space between repeats, in points. The cards tile by their *card box*
    /// (bleed box when `bleed_mode = Bleed`, else trim) packed tight from the centre — this is the
    /// inter-card gap, not an even gutter across the whole sheet.
    #[serde(default)]
    pub h_space_pt: Pt,
    #[serde(default)]
    pub v_space_pt: Pt,
    /// `Bleed` (default) tiles by the bleed box and shows each card's bleed; `NoBleed` tiles by the
    /// trim and clips to it.
    #[serde(default)]
    pub bleed_mode: BleedMode,
    /// Per-card scale. `None` = 100% / full size; `Fixed(f)` = `f`×. `Fit` is treated as 100%
    /// (fit-to-cell is meaningless when tiling at a fixed step).
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
    /// Optional spine *allowance* between the two pages of a spread, in points. **Defaults to 0**:
    /// the two pages butt at the fold (the booklet is pulled together). Non-zero only for unusual
    /// jobs that need a gap at the fold. This is **not** creep — creep (shingling: inner spreads
    /// shifted toward the spine to compensate for folded paper caliper) is a separate, optional
    /// refinement, deferred to v1 (the engine is zero-creep; SPEC §8.8/§10).
    #[serde(default)]
    pub spine_pt: Pt,
    /// Pull each page's bleed on its three outer (cut) edges; the spine edge is a fold and stays
    /// clipped to trim. Defaults to `NoBleed`.
    #[serde(default = "default_no_bleed")]
    pub bleed_mode: BleedMode,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PerfectBound {
    /// Pages per gathered signature (rounded up to a multiple of 4).
    pub signature_pages: u32,
    #[serde(default)]
    pub duplex: Duplex,
    #[serde(default)]
    pub scale: ScaleMode,
    /// Optional spine allowance per spread; **defaults to 0** (pages butt at the fold). See
    /// [`SaddleStitch::spine_pt`] — this is not creep.
    #[serde(default)]
    pub spine_pt: Pt,
    /// Spine-safe bleed-pull (outer edges only); see [`SaddleStitch::bleed_mode`]. Defaults to `NoBleed`.
    #[serde(default = "default_no_bleed")]
    pub bleed_mode: BleedMode,
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
