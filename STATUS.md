# Project Status — Very Good Document Imposer

> Internal engineering handoff (non-user-facing). Snapshot: 2026-06-18.
> Read this first when resuming with fresh context. Authoritative design lives in
> `SPEC.md`, `docs/adr/0001-platform-and-stack.md`, and `docs/m1-design.md`.

## What this is

A cross-platform, standalone **native** PDF imposition tool (alternative to Quite Imposing /
Montax / Imposition Studio / Kodak Preps). Name **Very Good Document Imposer**, distributor
**Cerebral Work Institute**, license **0BSD**. Aesthetic for the eventual GUI: tongue-in-cheek
2000s prepress (oxblood ground, humanist sans, routed-trace motif) — see the splash mock.

## Locked constraints (do not relitigate)

- Platforms: macOS + Windows + Linux. Native UI, **no Electron / no webview**. Not an Acrobat plugin.
- Licensing: **0BSD-maximal** — permissive deps only (qpdf Apache-2.0, PDFium BSD, lcms2 MIT,
  pure-Rust MIT/Apache). **No GPL/AGPL/LGPL** in the shipped graph (rules out MuPDF/Ghostscript/
  Poppler; forces native qpdf crypto, never GnuTLS).
- Surface: **engine/CLI first, GUI later**. Pro-prepress fidelity bar. **PDF-only** input for v1.
- Consequence: no permissive renderer does accurate CMYK soft-proof → future GUI gets honest RGB
  preview only.

## Architecture

Pure Rust headless engine → CLI; GUI deferred. `JobSpec` (JSON/TOML) → plan → imposed PDF.

```
crates/vgdi-types     domain model (JobSpec/scheme/sheet/marks). Pure serde, no PDF deps.
crates/vgdi-engine    geom · boxes · imposition (ordering) · marks (geometry) · plan
                      + qpdf_backend (behind `qpdf-backend` feature)
crates/vgdi-cli       `impose` binary (the automation contract)
spikes/spike0-qpdf    feasibility gate + `gen-fixture` (authors CMYK/TrimBox fixtures)
```

- **Core PDF primitive**: place each source page as a transformed **Form XObject** (never
  rasterize), assembled on the qpdf C-API object model (`new_stream(page_content)` + `/BBox`/
  `/Matrix`/`/Group` + `copy_from_foreign(resources)`), wrapped in an **isolated transparency
  group** carrying the blend `/CS`. qpdf's C++-only `getFormXObjectForPage` is NOT used.
- **Surfaces**: `PlannedSheet { surfaces: Vec<Surface> }`, `Surface { side: Front|Back, cells }`.
  The backend emits **one PDF page per surface**, so every scheme renders through one `Do` path.
- **Determinism** (hard invariant): `deterministic_id` + fixed object-stream/compression, no dates,
  stable number formatting → byte-identical output for a pinned vendored qpdf SHA.

## Milestone status

- **M0 (done):** N-up, TrimBox-anchor + ArtBox fallback + reject-on-absent, containment validation,
  `/Rotate` normalization, isolated-group wrapper, deterministic output, CLI.
- **M1 core (done — commit 541a5a2):**
  - Schemes: NUp, **StepRepeat** (spacing + bleed/no-bleed), **SaddleStitch**, **PerfectBound**
    (gathered signatures), **Manual** (per-surface custom placement).
  - Pure ordering (`engine::imposition`): `saddle_order`, `perfect_bound_order` (permutation-tested).
  - Placement: rotate-to-fit, fixed scale, duplex flip, manual mirror/rotate.
  - **Mark geometry** (`engine::marks`): crop (classic/full-line), center, registration (`All`),
    bleed outline — computed + tested. **Emission is NOT wired yet** (see next).
  - Full toggleable/customizable **MarkSet taxonomy** in `vgdi-types::marks` (crop/center/reg/fold/
    collation/color-bar/slug/autocutter/bleed/barcode/labels/…). Slug + Flip + `ScaleMode::Fixed`.
- **m1.5 (NEXT — the 3 ignored specs in `crates/vgdi-engine/tests/imposition_qpdf.rs`):**
  1. **Marks emission** — stroke the computed `MarkPrimitive`s into the sheet content stream in a
     **Separation colour space named `All`** (Type-2 tint transform → 100% CMYK). Test:
     `marks_emitted_use_separation_all_colorant`.
  2. **Bleed-pull** — render content out to the BleedBox band (gutter ≥ 2× bleed). Test:
     `bleed_pull_extends_visible_content_to_bleed`. (Step&repeat already switches the clip bbox;
     generalize to all schemes + collision rule.)
  3. **Slug text + barcodes** — emit slug fields/barcodes. Test: `slug_fields_render_text_in_slug_area`.
  - Also worth doing in m1.5: fold-mark geometry, collation back-step marks, color-bar geometry,
    `Flip` allowance rendering, perfector back-side 180° (SPEC §8.9), GWG equal-TrimBox flagging.

## Build / test / run

Requires Rust + a C++ toolchain. On this Mac the qpdf vendored build needs:
```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
export LIBCLANG_PATH=/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib
```
- Pure (fast, no qpdf): `cargo test -p vgdi-types -p vgdi-engine`
- Full (qpdf backend): `cargo test -p vgdi-engine --features qpdf-backend`
- Lint (CI gate): `cargo clippy --workspace --all-features -- -D warnings` and `cargo fmt --all --check`
- CLI: `cargo run -p vgdi-cli -- job.json -o out.pdf`  (see `examples/nup-2x2.json`)
- Fixtures: `cargo run -p spike0-qpdf --bin gen-fixture -- tests/fixtures/cmyk-trim.pdf 4`
- Ignored specs: `cargo test -p vgdi-engine --features qpdf-backend -- --ignored`

Test counts (M1): 44 passing (30 engine + 10 types + 4 integration) + 3 ignored emission specs.

## Gotchas / lessons (don't rediscover)

- `qpdf` crate is `ancwrd1/qpdf-rs` 0.3.5; `vendored` feature cc-compiles qpdf 12.0.0 + bundled
  zlib/jpeg + **native crypto** (keeps GnuTLS/LGPL out).
- `copy_from_foreign` requires an **indirect** object — promote direct `/Resources` via
  `into_indirect()` first or qpdf warns and may misbehave.
- Inheritable page attrs (`/MediaBox`, `/Rotate`, `/Resources`) may live on a `/Parent` Pages node;
  the reader walks `/Parent` (`get_inherited`).
- clap: a `///` doc comment renders as `--help` text. Copy gaps in CLI use plain `//` + `∑CG`.
- Copy rule: never author user-facing copy; mark gaps `∑CG` with a commented spec+sample. CLI help
  strings are provisional (one-time exception). Future-GUI copy gaps remain in `SPEC.md`.

## Open decisions (SPEC §18)

qpdf binding path (object-model — chosen) vs `qpdfjob-c.h`; GUI gate timing + toolkit (ADR-0002,
must be permissive — Slint is out); JDF/CIP3 priority; first-platform lead; signing recipient.

## CI

`.github/workflows/ci.yml`: 3-OS matrix (pure / spike0 vendored build / cargo-deny licenses /
fmt+clippy). Windows + Linux legs of Spike-0 are wired but **not yet confirmed green** (no runner
locally) — a Windows MSVC static-link failure is the trigger to evaluate the pure-Rust `lopdf` path.
