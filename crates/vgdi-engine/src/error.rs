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

    // --- Prepress-correctness rejections (SPEC §8) ---
    #[error("page {page} of source `{id}`: no TrimBox or ArtBox; non-conformant for pro prepress")]
    NoTrimOrArt { id: String, page: usize },

    #[error(
        "page {page} of source `{id}`: box containment violated (need Media ⊇ Bleed ⊇ Trim/Art)"
    )]
    ContainmentViolation { id: String, page: usize },

    // --- Backend / IO ---
    #[error("backend: {0}")]
    Backend(String),

    #[error("io: {0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, EngineError>;
