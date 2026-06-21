//! Engine error type. Display strings are terse technical diagnostics (not user-facing copy).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("jobspec schema mismatch: expected `{expected}`, got `{got}`")]
    SchemaMismatch { expected: String, got: String },

    #[error("scheme not supported in M0: `{0}` (only n-up)")]
    UnsupportedScheme(&'static str),

    #[error("jobspec references no sources")]
    NoSources,

    #[error("source `{0}` not found among loaded sources")]
    UnknownSource(String),

    #[error("source `{id}` has no pages")]
    EmptySource { id: String },

    #[error("n-up grid must be at least 1x1 (got {rows}x{cols})")]
    EmptyGrid { rows: u32, cols: u32 },

    #[error("sheet too small for grid + gutters + gripper (cell <= 0 in {axis})")]
    SheetTooSmall { axis: &'static str },

    #[error(
        "bleed-pull needs gutter ≥ 2× bleed ({needed:.3}pt) between neighbours, but gutter is {gutter:.3}pt"
    )]
    InsufficientBleedGutter { gutter: f64, needed: f64 },

    // --- M2 work styles / duplex back ---
    #[error("work style `{0}` not yet supported for a duplex gang/n-up back (M2 phase 2)")]
    WorkStyleUnsupported(&'static str),

    #[error(
        "back source `{back}` has {back_pages} page(s) but the front needs {front_pages} (1:1 pairing)"
    )]
    BackCountMismatch {
        back: String,
        back_pages: usize,
        front_pages: usize,
    },

    #[error(
        "back source `{back}` page {page}: trim/bleed geometry must match the paired front page (v1 requires equal size)"
    )]
    BackGeometryMismatch { back: String, page: usize },

    // --- Prepress-correctness rejections (SPEC §8) ---
    #[error("page {page} of source `{id}`: no TrimBox or ArtBox; non-conformant for pro prepress")]
    NoTrimOrArt { id: String, page: usize },

    #[error(
        "page {page} of source `{id}`: box containment violated (need Media ⊇ Bleed ⊇ Trim/Art)"
    )]
    ContainmentViolation { id: String, page: usize },

    #[error("page {page} of source `{id}`: /Rotate {rotate} is not a multiple of 90")]
    InvalidRotate {
        id: String,
        page: usize,
        rotate: i32,
    },

    // --- Backend / IO ---
    #[error("backend: {0}")]
    Backend(String),

    #[error("io: {0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, EngineError>;
