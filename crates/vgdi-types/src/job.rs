//! Root job specification + I/O-adjacent config.

use crate::marks::MarkSet;
use crate::scheme::Scheme;
use crate::sheet::Sheet;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Root job specification: the single source of truth (SPEC §7).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSpec {
    /// Must equal [`crate::SCHEMA_ID`].
    pub schema: String,
    pub sources: Vec<SourceRef>,
    pub scheme: Scheme,
    pub sheet: Sheet,
    #[serde(default)]
    pub marks: MarkSet,
    #[serde(default)]
    pub color_policy: ColorPolicy,
    #[serde(default)]
    pub output: OutputTarget,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRef {
    pub id: String,
    pub path: PathBuf,
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
    #[serde(default)]
    pub pdfx: Option<String>,
    #[serde(default)]
    pub pdf_version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheme::*;
    use crate::sheet::*;
    use crate::SCHEMA_ID;

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
                rotate_to_fit: false,
                bleed_mode: BleedMode::NoBleed,
            }),
            sheet: Sheet {
                size_pt: [595.276, 841.89],
                gripper_pt: 0.0,
                margin_pt: 0.0,
                work_style: WorkStyle::Sheetwise,
                flip: None,
            },
            marks: MarkSet::default(),
            color_policy: ColorPolicy::default(),
            output: OutputTarget::default(),
        };
        let s = serde_json::to_string(&job).unwrap();
        let back: JobSpec = serde_json::from_str(&s).unwrap();
        assert_eq!(back.schema, SCHEMA_ID);
        assert!(matches!(back.scheme, Scheme::NUp(_)));
    }

    #[test]
    fn minimal_jobspec_deserializes_with_defaults() {
        // marks/color_policy/output omitted -> defaults; scale defaults to fit.
        let json = r#"{
            "schema": "vgdi/jobspec@1",
            "sources": [{"id": "body", "path": "b.pdf"}],
            "scheme": {"kind": "saddle-stitch"},
            "sheet": {"size_pt": [800.0, 600.0]}
        }"#;
        let job: JobSpec = serde_json::from_str(json).unwrap();
        assert!(matches!(job.scheme, Scheme::SaddleStitch(_)));
        assert!(job.marks.crop.is_none());
        assert!(job.color_policy.preserve_spots);
    }

    #[test]
    fn scalemode_fixed_roundtrips() {
        let s = ScaleMode::Fixed(1.5);
        let j = serde_json::to_string(&s).unwrap();
        let back: ScaleMode = serde_json::from_str(&j).unwrap();
        assert_eq!(back, ScaleMode::Fixed(1.5));
    }
}
