# Project Status — Very Good Document Imposer

> Internal engineering handoff (non-user-facing). Snapshot: 2026-06-19.
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
    bleed outline — computed + tested.
  - Full toggleable/customizable **MarkSet taxonomy** in `vgdi-types::marks` (crop/center/reg/fold/
    collation/color-bar/slug/autocutter/bleed/barcode/labels/…). Slug + Flip + `ScaleMode::Fixed`.
- **M1.5 (done):** the three deferred emission specs are now wired + green (no `#[ignore]` left):
  1. **Marks emission** — `qpdf_backend` strokes/fills the planned primitives + slug text into the
     sheet content stream. Cut/reg marks draw in a **Separation `All`** colour space (`[/Separation
     /All /DeviceCMYK <Type-2 fn C0=[0000] C1=[1111] N=1>]`, declared per-page under
     `/Resources/ColorSpace/SepAll`); `Process` marks → DeviceCMYK `K`/`k`; named spots → per-job
     `/Spot{i}` Separations (placeholder 100%-K alternate). Test: `marks_emitted_use_separation_all_colorant`.
  2. **Bleed-pull** — N-up gained `bleed_mode` (default `NoBleed`); on `Bleed` the form `/BBox` is the
     BleedBox and a **gutter ≥ 2× placed-bleed** check (`InsufficientBleedGutter`) guards neighbour
     overlap. Step&repeat already switched the clip. Tests: `bleed_pull_extends_visible_content_to_bleed`,
     `bleed_pull_rejects_insufficient_gutter`.
  3. **Slug text + barcodes** — slug tokens resolve to a Helvetica text run (filename/sheet#/surface/
     custom; DateTime/operator/job#/separation skipped for determinism); Code 128-B encoder
     (`engine::barcode`) emits `job_barcode` as filled bars. Tests: `slug_fields_render_text_in_slug_area`,
     `job_barcode_emits_code128_bars`.
  - Also landed: **per-cell vs sheet-level** mark model (`MarkPlan` on `Surface`, `plan_surface_marks`):
    crop/centre/trim/bleed per placed page, registration/colour-bar/slug/fold/collation/barcode framed
    on the imposed content extent; **fold-mark**, **collation back-step**, **colour-bar** geometry;
    determinism verified (`marks_output_is_byte_deterministic`); gs cross-check render is clean.
  - **`sheet.margin_pt`** (imageable-area inset, all four edges) added after manual testing showed
    `Fit` grids fill the sheet edge-to-edge, clipping outer crop marks and overlapping sheet-edge
    furniture. The grid now lays out inside `[margin .. size−margin]` (+ gripper), reserving a mark
    band. `manual-tests/*.json` set it (32–40pt). Default 0 = fill the sheet (back-compat).
  - **Booklet mark correctness** (manual-testing finds): a booklet spread is one *folded leaf*, so
    (1) cut/trim marks frame the **whole leaf's outer perimeter** — no crop ticks at the spine (a
    fold, not a cut); on a half-blank (single-cell) spread the frame is reflected about the spine to
    recover the full leaf, so crop + registration land identically on full and half spreads. (2)
    **Collation back-step marks are perfect-bound only** — a saddle-stitch booklet is a single
    nested signature with nothing to collate. `SurfaceMarkInput.folded` + `reflect_about_x` carry
    this; the spine gets dashed **fold** marks instead. `manual-tests/04` shows it.
  - **Booklet bleed-pull (spine-safe)**: `SaddleStitch`/`PerfectBound` gained `bleed_mode`. On
    `Bleed`, each page pulls bleed on its **three outer (cut) edges** but keeps the **spine (fold)
    edge at trim** — so no ink crosses the fold into the facing page. The asymmetric clip is built in
    sheet space and mapped back through `Matrix::inverse` (handles `/Rotate` + duplex flip); no
    gutter check needed since nothing bleeds toward a neighbour. `gen-fixture … bleed` makes a
    BleedBox fixture (contrasting bleed band); `manual-tests/04` + `03a/b` show it.
  - **Still deferred** (geometry/types exist, emission/wiring pending): work-style reflections
    (perfector back-180°/work-and-turn/tumble), **GWG equal-TrimBox** flagging (needs a warnings
    channel), **Flip** allowance rendering, **hatched** bleed fill, cross-spine (reader-spread) bleed,
    QR/DataMatrix, embedded fonts (PDF/X). Known minor: colour-bar/barcode/slug all default to the
    Slug region → they stack; give them distinct `region`s, or a furniture layout pass is future work.

- **M1.6 (done — on branch `m1.5-emission`, NOT yet merged to `main`; commits 53a1d3b→66d050b):**
  Step & Repeat reworked into a "bulletproof" card-sheet gang, cross-checked byte-for-placement
  against **Quite Imposing** (and the booklet path vs QI on *the-metamorphosis.pdf* — exact CTM
  match `1.23 0 0 1.23 …`).
  - **True tight-pack** (was an even n-up grid): tile by the **card box** + spacing, block **centred**
    in the imageable area. `StepRepeat.rows/cols → max_rows/max_cols` (u32, **0 = fit as many as the
    sheet allows**, else cap). Helpers `fit_count` / `step_scale` in `plan_step_repeat`.
  - **Card box = bleed box** when `bleed_mode = Bleed` (default), so neighbours tile **bleed-to-bleed**:
    bleeds meet (no overlap, no white hairline), each trim sits one bleed inside → the two trim cut
    marks at an interior boundary are **one bleed apart (6mm for a 3mm bleed)**, cut line centred.
    `NoBleed` tiles by trim. (Earlier I tried tile-by-trim → 21 with overlapping bleeds; that's wrong
    for bleed marks. Correct count for the testcard on A3 is **3×6 = 18**.)
  - **Natural bleed by default** (QI parity): `PageBoxes::effective_bleed` = BleedBox ?? CropBox ??
    MediaBox, **capped to `MAX_NATURAL_BLEED_PT` (36pt)** per side so an oversized artboard isn't read
    as a giant bleed (review finding). A strict BleedBox-or-trim **toggle is still TODO**.
  - **Gang crop marks** (`marks::SurfaceLayout {Independent, Folded, Gang}`, replaced the `folded`
    bool): Gang draws crop ticks **only on the gang's outer perimeter**, one per distinct cut line,
    pushed clear of the bleed (never on the print surface / between cards). `gang_crop_marks` +
    `cluster_coords`. **Single shared cut mark when the file has no bleed; double (each card's trim)
    when it has bleed** — `has_bleed` per cell; cluster window = ½ the smallest card dim.
  - Adversarial review (workflow) confirmed + we fixed: unbounded media-as-bleed (→ cap), crop-offset
    off-sheet (→ cap + perimeter marks), per-card ticks invading neighbours (→ gang perimeter+dedup).
  - ⚠️ **Workflow hazard learned twice:** review-workflow subagents run `git checkout` and **wipe
    uncommitted changes** (plan.rs got reverted mid-task). **Commit before launching any workflow.**
  - `manual-tests/testcard-steprepeat.json` (uses `/Users/ruby/Downloads/testcard.pdf`, 85×55mm card,
    3mm bleed) and `metamorphosis-booklet.json` are the QI reference jobs.

- **NEXT JOB — optional inner-bleed "creep" (make this work perfectly):** let the user tighten the
  gang by **cropping the shared inner bleed**, gaining rows/cols. Confirmed design with the owner:
  - **Default stays FULL inner bleed** (bleed-to-bleed, 6mm between trims). We deliberately differ
    from QI/Fiery, which **creep to half bleed by default**.
  - Opt-in via the JobSpec / formula-script / future UI, e.g. *"creep to half bleed"* (fraction) or
    *"inner combined bleed = 4mm"* (absolute total band between the two trims). Fiery Impose has this.
  - **Mechanism:** clip each card's **inner-facing** bleed edges shorter (to the requested amount);
    cards step closer by the cropped amount; the cut/meeting line stays **centred between the two trim
    marks**; **outer perimeter keeps full bleed**; trim crop marks move inward with the cards. This is
    the booklet spine-safe-clip idea generalised to grid inner edges (reuse `Matrix::inverse`; per
    cell decide which edges are shared/interior vs outer/perimeter from its grid position).
  - **Yield proof:** testcard full-bleed = 1210pt tall → 6 rows (18). Shave ≈0.6mm off each inner
    bleed → <1190pt → **7 rows → 21** (== QI). So creep is exactly how QI reaches 21.
  - **Where:** add an inner-bleed field to `StepRepeat` (e.g. `InnerBleed { Full | Fraction(f64) |
    CombinedPt(f64) }`, default `Full`); step = `card_box − 2×creep` per axis; asymmetric per-cell
    clip for inner vs outer edges; gang crop marks already follow the (moved) trims.

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
- Dev-only PDF cross-check (not shipped; SPEC §13): `gs -dBATCH -dNOPAUSE -sDEVICE=nullpage out.pdf`.

Test counts (M1.6): 80 passing (60 engine + 10 types + 10 integration), 0 ignored. Two adversarial
multi-agent reviews drove fixes across M1.5/M1.6 (slug `/WinAnsiEncoding`, 4-side crop clamp,
collation spine anchor, capped natural bleed, gang perimeter/de-dup marks). All have regression tests.

## Gotchas / lessons (don't rediscover)

- `qpdf` crate is `ancwrd1/qpdf-rs` 0.3.5; `vendored` feature cc-compiles qpdf 12.0.0 + bundled
  zlib/jpeg + **native crypto** (keeps GnuTLS/LGPL out).
- `copy_from_foreign` requires an **indirect** object — promote direct `/Resources` via
  `into_indirect()` first or qpdf warns and may misbehave.
- Inheritable page attrs (`/MediaBox`, `/Rotate`, `/Resources`) may live on a `/Parent` Pages node;
  the reader walks `/Parent` (`get_inherited`).
- clap: a `///` doc comment renders as `--help` text. Copy gaps in CLI use plain `//` + `∑CG`.
- qpdf writes dictionary keys **sorted** (not insertion order), so building resource/colorspace
  dicts in any order stays byte-deterministic — confirmed by `marks_output_is_byte_deterministic`.
- Mark content ops are emitted in **sheet space** (no CTM) appended after the cells' `Do` calls,
  wrapped in `q … Q`; numbers go through `geom::fmt` for stable formatting.
- Slug must **never** emit wall-clock (`DateTime`) — it would break byte-determinism; those tokens
  are skipped. Slug content is operator data (filename/sheet#/surface), not authored copy.
- Copy rule: never author user-facing copy; mark gaps `∑CG` with a commented spec+sample. CLI help
  strings are provisional (one-time exception). Future-GUI copy gaps remain in `SPEC.md`.

## Open decisions (SPEC §18)

qpdf binding path (object-model — chosen) vs `qpdfjob-c.h`; GUI gate timing + toolkit (ADR-0002,
must be permissive — Slint is out); JDF/CIP3 priority; first-platform lead; signing recipient.

## CI

`.github/workflows/ci.yml`: 3-OS matrix (pure / spike0 vendored build / cargo-deny licenses /
fmt+clippy). Windows + Linux legs of Spike-0 are wired but **not yet confirmed green** (no runner
locally) — a Windows MSVC static-link failure is the trigger to evaluate the pure-Rust `lopdf` path.
