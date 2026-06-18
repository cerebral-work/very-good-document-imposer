//! Very Good Document Imposer — headless imposition engine.
//!
//! Pipeline: parse `JobSpec` -> read source page geometry -> [`plan`] -> emit imposed PDF.
//! The planner and geometry kernel are pure and PDF-backend-independent; the actual PDF read/
//! write lives behind the `qpdf-backend` feature so the core compiles and unit-tests without
//! the vendored C++ build (SPEC §5).

pub mod boxes;
pub mod error;
pub mod geom;
pub mod plan;

#[cfg(feature = "qpdf-backend")]
pub mod qpdf_backend;

pub use error::{EngineError, Result};
pub use plan::{plan, Cell, GroupCs, ImpositionPlan, PageGeometry, PlannedSheet, SourceInfo};

/// Read sources referenced by `job`, plan, and write the imposed PDF bytes to `out`.
/// (qpdf backend only.)
#[cfg(feature = "qpdf-backend")]
pub fn impose_to_file(job: &vgdi_types::JobSpec, out: &std::path::Path) -> Result<()> {
    qpdf_backend::impose_to_file(job, out)
}
