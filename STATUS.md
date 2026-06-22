# Project Status — Very Good Document Imposer

> Internal engineering handoff (non-user-facing). Snapshot: 2026-06-21.
> Read this first when resuming with fresh context. Authoritative design lives in
> `SPEC.md`, `docs/adr/0001-platform-and-stack.md`, `docs/m1-design.md`, and `docs/m2-design.md`.

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

- **M1.6 (done — merged to `main`; commits 53a1d3b→66d050b):**
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

- **M1.7 — inner-bleed "creep" (done — merged to `main`, commits 2a32b3d→1ab372b):** the user
  can tighten the gang by **cropping the shared inner bleed**, gaining rows/cols. Validated to exact
  **QI parity**: the testcard on A3 is `3×6 = 18` full-bleed and `3×7 = 21` at creep-to-half-bleed.
  - **`StepRepeat.inner_bleed: InnerBleed { Full | Fraction(f64) | CombinedPt(f64) }`**, default
    `Full` (serde-tagged like `ScaleMode`: `"full"` / `{"fraction":0.5}` / `{"combined-pt":11.34}`;
    omitting it is backward-compatible). **Default stays FULL inner bleed** — we deliberately differ
    from QI/Fiery, which creep to half by default. `Fraction(f)` keeps fraction `f` of the inner
    bleed (0.5 = "creep to half bleed"); `CombinedPt(t)` sets the combined band between two trims.
  - **Mechanism** (`plan_step_repeat`): inner-facing bleed edges are cropped so the cards step closer
    (pitch shrinks by the cropped amount), the **outer perimeter keeps full bleed**, and each cut line
    stays **centred between its two trims**. The asymmetric clip is built in sheet space and mapped to
    page space via `Matrix::inverse` (booklet spine-safe-clip generalised to grid inner edges); per
    cell, neighbour-facing edges are chosen from grid position. `Full`/`NoBleed` keep the bbox = card
    box exactly (byte-identical to M1.6). Gang crop marks follow the moved trims automatically.
  - **Per-edge, not averaged** (review fix, 1ab372b): `inner_creeps` measures each side's *true* bleed
    via a probe placement (rotation-correct) so an **off-centre trim** (asymmetric BleedBox — a legal
    input) still lands the bleed seam on the centred cut instead of `(bR−bL)/2` past it. Reduces to the
    old symmetric creep when `bL==bR` (all real jobs). Non-finite `Fraction`/`CombinedPt` → `Full`.
  - Adversarial review (workflow, again) drove the per-edge fix + the NaN guard; both have regression
    tests (`…asymmetric_bleed_keeps_seam_on_centred_cut`, `inner_creeps_non_finite_falls_back_to_full`).
  - `manual-tests/testcard-steprepeat-creep.json` is the creep reference job (`fraction: 0.5` → 21).

- **M2 work styles — Phases 1 + 2a + 2b (done, branch `m2-work-styles`, not yet merged).** Design in
  `docs/m2-design.md` (revised after a 5-lens adversarial workflow review: 16 confirmed findings
  folded in — re-scoped phases, decided the back-source model, scoped the marks claim, fixed the test
  oracles). `WorkStyle` now drives a **duplex back surface** on N-up + Step & Repeat (was inert; both
  paths were front-only via `one_surface_sheet`, now replaced by `duplex_sheet`).
  - **All four styles wired:** **Sheetwise** (back on its own independent grid, same positions),
    **WorkAndTurn** (reflect about the vertical centreline), **WorkAndTumble** (reflect about the
    horizontal centreline), **Perfector** (180° about the sheet centre + content rotated 180°). All
    verified end-to-end through the CLI (`manual-tests/m2-work-styles/`, gs-rendered PNGs).
    `work_style` stays **inert unless a `back` is configured** (back-compat).
  - **Furniture relocates with the gripper (2b):** `SurfaceMarkInput.gripper_edge` (`Bottom`/`Top`);
    `attach_marks` sets the back of a gang/N-up tumble/perfector job to `Top`, and
    `reflect_furniture_to_top` mirrors slug/colour-bar/barcode about the sheet centre (glyphs upright)
    so they park just inside the moved gripper, never in the bite. Cell-derived marks (crop/centre/
    trim/registration) reflect with the cells. The 2a `FurnitureOnMovedGripper` guard is **gone** — all
    four styles take any mark set. (A *user-configurable* sheet gripper edge + Left/Right + `grid_cell_rect`
    generalisation is future; work styles only move bottom↔top.)
  - **Data model:** `StepRepeat.back` / `NUp.back: Option<BackSpec>` (`BackSpec { source }` = a second
    declared source, 1:1 by fill order). **v1 requires equal trim+bleed geometry** (else
    `BackGeometryMismatch`) and equal page count (`BackCountMismatch`) — so the front-derived
    scale/gutter/inner-bleed-creep carries to the back unchanged and the cut registers.
  - **Mechanism:** `work_style_reflect` (the `T` position transform) + `geom::reflect_x`/`reflect_y`;
    back content placed via `place_best` — upright for turn/tumble, `flip180` for perfector — never a
    content mirror, so **det(CTM) > 0** on every cell (asserted across all four styles). The back clip
    reflects the front clip through `T` in sheet space and maps back via `Matrix::inverse` (booklet
    spine-safe-clip generalised → inner creep carries over for free). Cell-derived marks follow the
    reflected cells automatically.
  - **Tests (pure, no qpdf):** per-style position reflection (turn/tumble/perfector) + involution,
    perfector content-180° (`a<0`) + det>0 across all styles, sheetwise same-positions, moved-gripper
    back furniture relocates to the top (turn/sheetwise stays bottom), work-style-inert-without-back,
    count/geometry mismatch rejections, `/Rotate`-on-back, `reflect_x`/`reflect_y` involution. **All
    green: 77 engine + 10 integration + 11 types; clippy `--all-features` + fmt clean.** All four styles
    (incl. tumble+slug furniture relocation) verified end-to-end through the CLI.
  - **Phase 2c (partial):** backend integration coverage added
    (`imposition_qpdf.rs::duplex_nup_renders_front_and_back_surfaces_deterministically` — two
    surfaces, byte-deterministic, back content ≠ front). **77 engine + 11 integration + 11 types.**
    *Remaining (owner-side):* literal byte/CTM parity vs a Quite Imposing reference export (needs the
    QI files); `manual-tests` jobs with real distinct front/back art (current fixtures are identical).
  - **NEXT:** **Phase 3** — make `work_style` meaningful on the booklet paths (mostly metadata, since a
    symmetric 2-up turn ≡ sheetwise plate). Then merge `m2-work-styles` → `main` (CI runs the 3-OS
    matrix + cargo-deny + fmt/clippy).
- **Still on the backlog (smaller / infra):** (a) **strict BleedBox-or-trim toggle** (the M1.6 still-
  TODO: a job flag to disable natural-bleed inference so only an explicit BleedBox counts); (b) the
  **warnings channel** (GWG equal-TrimBox flags; also wanted by M2 Phase 3); (c) other deferred
  furniture/colour from M1.5 "Still deferred": hatched bleed fill, cross-spine (reader-spread) bleed,
  QR/DataMatrix, `Flip` allowance rendering.

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

Test counts (M1.7): 85 passing (64 engine + 11 types + 10 integration), 0 ignored. Three adversarial
multi-agent reviews drove fixes across M1.5/M1.6/M1.7 (slug `/WinAnsiEncoding`, 4-side crop clamp,
collation spine anchor, capped natural bleed, gang perimeter/de-dup marks, per-edge creep on
asymmetric bleed, non-finite creep guard). All have regression tests.

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
fmt+clippy). **All 8 jobs confirmed GREEN on GitHub Actions** (first run after publishing to
`cerebral-work/very-good-document-imposer`, 2026-06-20). **Vendored qpdf static-links cleanly on
Windows MSVC + Ubuntu + macOS** — the documented Windows-MSVC-static-link risk did NOT materialise,
so the pure-Rust `lopdf` fallback (SPEC §16 / ADR-0001 trigger) is **not needed**; the qpdf object-
model path is validated cross-platform. (First green run required a one-line fix: the `spike0-qpdf`
crate ships two binaries, so CI's `cargo run -p spike0-qpdf` was ambiguous and errored *before*
building qpdf — fixed with `default-run = "spike0-qpdf"`, commit 2b56c8f.)
