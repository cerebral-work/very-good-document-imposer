//! Printer-marks taxonomy (M1 design). Every mark family is individually toggleable (an
//! `Option`/`bool` on [`MarkSet`]) and most are customizable. `MarkSet::default()` emits nothing.
//!
//! Geometry computation for the core families is implemented in `vgdi-engine::marks`; PDF
//! emission of marks is a deferred milestone (executable `#[ignore]` specs).

use crate::Pt;
use serde::{Deserialize, Serialize};

/// The complete set of marks/furniture requested for a job.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MarkSet {
    // --- cut / trim ---
    pub crop: Option<CropMarks>,
    pub center: Option<CenterMarks>,
    pub trim_outline: bool,

    // --- registration / alignment ---
    pub registration: Option<RegistrationMarks>,
    pub star_targets: bool,
    pub side_lay: bool,
    pub punch_register: bool,
    pub vision_marks: bool,

    // --- fold / bind ---
    pub fold: Option<FoldMarks>,
    pub spine: bool,
    pub collation: Option<CollationMarks>,
    pub perforation: bool,

    // --- colour / quality control ---
    pub color_bar: Option<ColorBar>,
    pub gray_balance: bool,
    pub dot_gain_targets: bool,
    pub ink_key_zones: bool,

    // --- cutting automation ---
    pub autocutter: Option<AutocutterMarks>,

    // --- information / slug ---
    pub slug: Option<Slug>,
    pub job_barcode: Option<Barcode>,
    pub page_labels: bool,
    pub station_numbers: bool,

    // --- other ---
    pub bleed: BleedTreatment,
    pub gripper_indicator: bool,
    pub bearer_bars: bool,
    pub id_dots: bool,
}

/// Where a mark colorant draws.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarkColor {
    /// Separation colorant `All` — appears on every plate (the correct default for cut/reg marks).
    #[default]
    RegistrationAll,
    /// A process tint, 0..1 per channel.
    Process { c: f64, m: f64, y: f64, k: f64 },
    /// A named spot colorant.
    Spot(String),
}

/// A sheet region marks can be parked in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarkRegion {
    #[default]
    Slug,
    Head,
    Foot,
    Left,
    Right,
}

// ----------------------------------------------------------------------------- cut / trim

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CropMarks {
    pub style: CropStyle,
    pub length_pt: Pt,
    /// Outward offset from the trim corner; must be ≥ the bleed amount.
    pub offset_pt: Pt,
    pub weight_pt: Pt,
    pub color: MarkColor,
}

impl Default for CropMarks {
    fn default() -> Self {
        CropMarks {
            style: CropStyle::Classic,
            length_pt: 14.173, // ~5 mm
            offset_pt: 8.504,  // ~3 mm (typical bleed)
            weight_pt: 0.25,
            color: MarkColor::RegistrationAll,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CropStyle {
    /// Offset corner ticks (the common style).
    #[default]
    Classic,
    /// Lines spanning across the sheet through the trim corners.
    FullLine,
    /// Japanese/double marks: an inner pair at trim and an outer pair at bleed.
    Japanese,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CenterMarks {
    pub length_pt: Pt,
    pub weight_pt: Pt,
    pub color: MarkColor,
}

impl Default for CenterMarks {
    fn default() -> Self {
        CenterMarks {
            length_pt: 14.173,
            weight_pt: 0.25,
            color: MarkColor::RegistrationAll,
        }
    }
}

// ------------------------------------------------------------------- registration / alignment

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RegistrationMarks {
    pub positions: RegPositions,
    pub diameter_pt: Pt,
    pub weight_pt: Pt,
}

impl Default for RegistrationMarks {
    fn default() -> Self {
        RegistrationMarks {
            positions: RegPositions::EdgeCenters,
            diameter_pt: 9.0,
            weight_pt: 0.25,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegPositions {
    #[default]
    EdgeCenters,
    Corners,
    All,
}

// ---------------------------------------------------------------------------- fold / bind

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct FoldMarks {
    pub length_pt: Pt,
    pub weight_pt: Pt,
    /// Dash length (0 = solid).
    pub dash_pt: Pt,
    pub color: MarkColor,
}

impl Default for FoldMarks {
    fn default() -> Self {
        FoldMarks {
            length_pt: 14.173,
            weight_pt: 0.25,
            dash_pt: 3.0,
            color: MarkColor::RegistrationAll,
        }
    }
}

/// Back-step collation marks (perfect-bind signature sequencing).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CollationMarks {
    pub width_pt: Pt,
    pub height_pt: Pt,
    pub color: MarkColor,
}

impl Default for CollationMarks {
    fn default() -> Self {
        CollationMarks {
            width_pt: 4.0,
            height_pt: 9.0,
            color: MarkColor::RegistrationAll,
        }
    }
}

// ------------------------------------------------------------------- colour / quality control

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorBar {
    pub kind: ColorBarKind,
    pub patch_pt: Pt,
    pub region: MarkRegion,
}

impl Default for ColorBar {
    fn default() -> Self {
        ColorBar {
            kind: ColorBarKind::ProcessSolids,
            patch_pt: 9.0,
            region: MarkRegion::Slug,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorBarKind {
    #[default]
    ProcessSolids,
    TintRamp,
    SpotPatches,
}

// -------------------------------------------------------------------- cutting automation

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AutocutterMarks {
    pub system: CutterSystem,
    pub barcode: bool,
    pub cut_marks: bool,
    pub omr: bool,
}

impl Default for AutocutterMarks {
    fn default() -> Self {
        AutocutterMarks {
            system: CutterSystem::Generic,
            barcode: true,
            cut_marks: true,
            omr: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CutterSystem {
    #[default]
    Generic,
    Polar,
    Wohlenberg,
}

// ------------------------------------------------------------------- information / slug

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Slug {
    pub position: SlugPosition,
    pub fields: Vec<SlugField>,
    pub font_pt: Pt,
}

impl Default for Slug {
    fn default() -> Self {
        Slug {
            position: SlugPosition::BottomLeft,
            fields: vec![
                SlugField::FileName,
                SlugField::SheetNumber,
                SlugField::Surface,
            ],
            font_pt: 6.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SlugPosition {
    #[default]
    BottomLeft,
    BottomCenter,
    BottomRight,
    TopLeft,
    TopCenter,
    TopRight,
}

/// Slug content tokens. `Custom` carries user-supplied literal text (data, not authored copy).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SlugField {
    FileName,
    DateTime,
    SheetNumber,
    Surface,
    Separation,
    Operator,
    JobNumber,
    Custom(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Barcode {
    pub symbology: Symbology,
    /// Encoded payload (job-supplied data).
    pub data: String,
    pub region: MarkRegion,
}

impl Default for Barcode {
    fn default() -> Self {
        Barcode {
            symbology: Symbology::Code128,
            data: String::new(),
            region: MarkRegion::Slug,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Symbology {
    #[default]
    Code128,
    Qr,
    DataMatrix,
}

// ----------------------------------------------------------------------------------- other

/// Treatment of the bleed band between TrimBox and BleedBox.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum BleedTreatment {
    #[default]
    None,
    Outline {
        weight_pt: Pt,
    },
    Hatched {
        spacing_pt: Pt,
        angle_deg: f64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_markset_is_all_off() {
        let m = MarkSet::default();
        assert!(m.crop.is_none());
        assert!(m.registration.is_none());
        assert!(!m.star_targets);
        assert!(matches!(m.bleed, BleedTreatment::None));
    }

    #[test]
    fn crop_marks_default_offset_covers_typical_bleed() {
        let c = CropMarks::default();
        // offset must be ≥ a 3 mm bleed so marks aren't overprinted by bleeding content.
        assert!(c.offset_pt >= 8.5);
        assert!(matches!(c.style, CropStyle::Classic));
    }

    #[test]
    fn markset_partial_json_toggles_only_named_marks() {
        let json =
            r#"{ "crop": {}, "registration": { "positions": "corners" }, "star_targets": true }"#;
        let m: MarkSet = serde_json::from_str(json).unwrap();
        assert!(m.crop.is_some());
        assert_eq!(m.registration.unwrap().positions, RegPositions::Corners);
        assert!(m.star_targets);
        assert!(m.fold.is_none());
    }

    #[test]
    fn bleed_treatment_tagged_roundtrip() {
        let b = BleedTreatment::Hatched {
            spacing_pt: 6.0,
            angle_deg: 45.0,
        };
        let j = serde_json::to_string(&b).unwrap();
        let back: BleedTreatment = serde_json::from_str(&j).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn slug_custom_field_carries_literal_data() {
        let s = SlugField::Custom("job-4471".into());
        let j = serde_json::to_string(&s).unwrap();
        let back: SlugField = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
