//! The pure planner: `JobSpec` + source page geometry -> `ImpositionPlan`.
//!
//! Deterministic and PDF-backend-independent. The plan is the intermediate the (future) GUI
//! previews and the writer serializes (SPEC §7 ImpositionPlan).

use crate::boxes::PageBoxes;
use crate::error::{EngineError, Result};
use crate::geom::{place_trim_in_cell, Matrix};
use vgdi_types::{FillOrder, JobSpec, Rect, Scheme, SCHEMA_ID};

/// Geometry of one source page needed for planning.
#[derive(Clone, Copy, Debug)]
pub struct PageGeometry {
    pub boxes: PageBoxes,
    /// PDF `/Rotate`, normalized to one of 0/90/180/270 by the reader.
    pub rotate: i32,
}

/// All pages of one input source.
#[derive(Clone, Debug)]
pub struct SourceInfo {
    pub id: String,
    pub pages: Vec<PageGeometry>,
}

/// Blend color space assigned to a placed page's isolated transparency-group wrapper
/// (SPEC §8 invariant #4). M0 defaults to DeviceCMYK (the device/OutputIntent space) when the
/// source page declares no group color space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupCs {
    DeviceCmyk,
    DeviceRgb,
    DeviceGray,
}

impl GroupCs {
    pub fn pdf_name(&self) -> &'static str {
        match self {
            GroupCs::DeviceCmyk => "DeviceCMYK",
            GroupCs::DeviceRgb => "DeviceRGB",
            GroupCs::DeviceGray => "DeviceGray",
        }
    }
}

/// One placed source page on a surface.
#[derive(Clone, Debug)]
pub struct Cell {
    pub source_id: String,
    pub source_page: usize,
    /// Painting CTM: `q <ctm> cm /Xn Do Q`.
    pub ctm: Matrix,
    /// Form `/BBox` (the source trim box), clipping in page space.
    pub bbox: Rect,
    /// Blend color space for the isolated wrapper group.
    pub group_cs: GroupCs,
}

/// One output sheet. M0 emits a single surface (front) per sheet.
#[derive(Clone, Debug)]
pub struct PlannedSheet {
    pub width: f64,
    pub height: f64,
    pub cells: Vec<Cell>,
}

/// The deterministic computed imposition result.
#[derive(Clone, Debug)]
pub struct ImpositionPlan {
    pub sheets: Vec<PlannedSheet>,
}

/// Plan an imposition. M0: single-source N-up across as many sheets as needed.
pub fn plan(job: &JobSpec, sources: &[SourceInfo]) -> Result<ImpositionPlan> {
    if job.schema != SCHEMA_ID {
        return Err(EngineError::SchemaMismatch {
            expected: SCHEMA_ID.to_string(),
            got: job.schema.clone(),
        });
    }
    if job.sources.is_empty() || sources.is_empty() {
        return Err(EngineError::NoSources);
    }

    let nup = match &job.scheme {
        Scheme::NUp(n) => n,
        Scheme::StepRepeat(_) => return Err(EngineError::UnsupportedScheme("step-repeat")),
        Scheme::Booklet(_) => return Err(EngineError::UnsupportedScheme("booklet")),
    };
    if nup.rows == 0 || nup.cols == 0 {
        return Err(EngineError::EmptyGrid {
            rows: nup.rows,
            cols: nup.cols,
        });
    }

    // M0 consumes the first source only.
    let src = &sources[0];
    if src.pages.is_empty() {
        return Err(EngineError::EmptySource { id: src.id.clone() });
    }

    let (sheet_w, sheet_h) = (job.sheet.size_pt[0], job.sheet.size_pt[1]);
    let gripper = job.sheet.gripper_pt;
    let gutter = nup.gutter_pt;
    let (rows, cols) = (nup.rows, nup.cols);

    // Usable area = sheet minus the gripper margin reserved on the bottom (gripper) edge.
    let usable_w = sheet_w;
    let usable_h = sheet_h - gripper;
    let cell_w = (usable_w - gutter * (cols as f64 - 1.0)) / cols as f64;
    let cell_h = (usable_h - gutter * (rows as f64 - 1.0)) / rows as f64;
    if cell_w <= 0.0 {
        return Err(EngineError::SheetTooSmall { axis: "x" });
    }
    if cell_h <= 0.0 {
        return Err(EngineError::SheetTooSmall { axis: "y" });
    }

    let per_sheet = (rows * cols) as usize;
    let mut sheets = Vec::new();
    let mut cells: Vec<Cell> = Vec::new();

    for (page_idx, geom) in src.pages.iter().enumerate() {
        // Validate prepress invariants (reject, never misplace).
        let trim = geom
            .boxes
            .effective_trim()
            .ok_or_else(|| EngineError::NoTrimOrArt {
                id: src.id.clone(),
                page: page_idx,
            })?;
        if !geom.boxes.containment_ok() {
            return Err(EngineError::ContainmentViolation {
                id: src.id.clone(),
                page: page_idx,
            });
        }

        let slot = cells.len();
        let (r, c) = slot_to_rowcol(slot, rows, cols, nup.fill);
        let cell_rect = cell_rect(r, c, rows, gripper, cell_w, cell_h, gutter);
        let placement = place_trim_in_cell(trim, geom.rotate, cell_rect, nup.scale);

        cells.push(Cell {
            source_id: src.id.clone(),
            source_page: page_idx,
            ctm: placement.ctm,
            bbox: placement.bbox,
            // M0: assume device CMYK blend space (SPEC §8 #4 default).
            group_cs: GroupCs::DeviceCmyk,
        });

        if cells.len() == per_sheet {
            sheets.push(PlannedSheet {
                width: sheet_w,
                height: sheet_h,
                cells: std::mem::take(&mut cells),
            });
        }
    }
    if !cells.is_empty() {
        sheets.push(PlannedSheet {
            width: sheet_w,
            height: sheet_h,
            cells,
        });
    }

    Ok(ImpositionPlan { sheets })
}

/// Map a fill slot index to a (row, col), row 0 at the top.
fn slot_to_rowcol(slot: usize, rows: u32, cols: u32, fill: FillOrder) -> (u32, u32) {
    let slot = slot as u32 % (rows * cols);
    match fill {
        FillOrder::RowMajor => (slot / cols, slot % cols),
        FillOrder::ColMajor => (slot % rows, slot / rows),
    }
}

/// Compute the rectangle for cell (row, col). Row 0 is the top row; the gripper margin is at
/// the bottom, so lower rows sit higher in user space.
fn cell_rect(
    r: u32,
    c: u32,
    rows: u32,
    gripper: f64,
    cell_w: f64,
    cell_h: f64,
    gutter: f64,
) -> Rect {
    let llx = c as f64 * (cell_w + gutter);
    // top row (r=0) is highest; convert to a bottom-origin y.
    let from_top = r as f64;
    let lly = gripper + (rows as f64 - 1.0 - from_top) * (cell_h + gutter);
    Rect::new(llx, lly, llx + cell_w, lly + cell_h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vgdi_types::{ColorPolicy, NUp, OutputTarget, ScaleMode, Sheet, SourceRef, WorkStyle};

    fn geom(trim: Option<Rect>, rotate: i32) -> PageGeometry {
        PageGeometry {
            boxes: PageBoxes {
                media: Rect::new(0.0, 0.0, 200.0, 200.0),
                crop: None,
                trim,
                art: None,
                bleed: None,
            },
            rotate,
        }
    }

    fn job_2x2(scale: ScaleMode) -> JobSpec {
        JobSpec {
            schema: SCHEMA_ID.to_string(),
            sources: vec![SourceRef {
                id: "body".into(),
                path: "body.pdf".into(),
            }],
            scheme: Scheme::NUp(NUp {
                rows: 2,
                cols: 2,
                fill: FillOrder::RowMajor,
                scale,
                gutter_pt: 0.0,
            }),
            sheet: Sheet {
                size_pt: [800.0, 800.0],
                gripper_pt: 0.0,
                work_style: WorkStyle::Sheetwise,
            },
            color_policy: ColorPolicy::default(),
            output: OutputTarget::default(),
        }
    }

    #[test]
    fn five_pages_2x2_makes_two_sheets() {
        let job = job_2x2(ScaleMode::Fit);
        let pages: Vec<_> = (0..5)
            .map(|_| geom(Some(Rect::new(10.0, 10.0, 190.0, 190.0)), 0))
            .collect();
        let sources = vec![SourceInfo {
            id: "body".into(),
            pages,
        }];
        let plan = plan(&job, &sources).unwrap();
        assert_eq!(plan.sheets.len(), 2);
        assert_eq!(plan.sheets[0].cells.len(), 4);
        assert_eq!(plan.sheets[1].cells.len(), 1);
    }

    #[test]
    fn page_without_trim_or_art_is_rejected() {
        let job = job_2x2(ScaleMode::Fit);
        let sources = vec![SourceInfo {
            id: "body".into(),
            pages: vec![geom(None, 0)],
        }];
        let err = plan(&job, &sources).unwrap_err();
        assert!(matches!(err, EngineError::NoTrimOrArt { .. }));
    }

    #[test]
    fn unsupported_scheme_rejected() {
        let mut job = job_2x2(ScaleMode::Fit);
        job.scheme = Scheme::Booklet(vgdi_types::Booklet {
            duplex: vgdi_types::Duplex::LongEdge,
        });
        let sources = vec![SourceInfo {
            id: "body".into(),
            pages: vec![geom(Some(Rect::new(10.0, 10.0, 190.0, 190.0)), 0)],
        }];
        assert!(matches!(
            plan(&job, &sources).unwrap_err(),
            EngineError::UnsupportedScheme("booklet")
        ));
    }

    #[test]
    fn cells_are_distinct_positions() {
        let job = job_2x2(ScaleMode::None);
        let pages: Vec<_> = (0..4)
            .map(|_| geom(Some(Rect::new(0.0, 0.0, 100.0, 100.0)), 0))
            .collect();
        let sources = vec![SourceInfo {
            id: "body".into(),
            pages,
        }];
        let plan = plan(&job, &sources).unwrap();
        let sheet = &plan.sheets[0];
        // 2x2 on an 800x800 sheet -> cell origins at the four quadrants (centered placement).
        let mut origins: Vec<(i64, i64)> = sheet
            .cells
            .iter()
            .map(|cell| {
                let (x, y) = cell.ctm.apply(cell.bbox.llx, cell.bbox.lly);
                (x.round() as i64, y.round() as i64)
            })
            .collect();
        origins.sort();
        origins.dedup();
        assert_eq!(
            origins.len(),
            4,
            "all four cells must occupy distinct positions"
        );
    }
}
