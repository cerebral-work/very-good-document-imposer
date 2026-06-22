# M2 Design — work styles (sheetwise / work-and-turn / work-and-tumble / perfector)

> Status: Draft · 2026-06-21 · Internal (non-user-facing). Extends `SPEC.md` §9 (work styles) /
> §10. Companion to `docs/m1-design.md`. Built TDD like M1: pure planner/geometry first (no qpdf),
> backend emission second.

## Scope

Make the press **work style** actually drive imposition, and add the duplex back surface it needs to
be useful. Today `WorkStyle` is stored on `Sheet` but **inert** — nothing in the engine reads it —
and the only schemes that emit a back surface are the booklet paths. The marquee capability M2 adds
is **duplex gang / N-up back-printing** (e.g. business cards with a distinct back), positioned per
work style so the back registers to the front after the press flip.

**Out of M2 scope:** SPEC's glossary (§ "Work style") lists a fifth style, **come-and-go**; it is
*not* in the `WorkStyle` enum and is *not* implemented here — it overlaps cut-and-stack (a separate
deferred scheme). Deliberate cut, not an oversight. SPEC §9's normative text specifies only the four
styles below.

## Verified current state (the starting line)

- **`WorkStyle` is inert.** The enum (`Sheetwise | Perfector | WorkAndTurn | WorkAndTumble`,
  `vgdi-types::sheet`) is only ever defaulted to `Sheetwise` (`job.rs`, `plan.rs`, tests). No match
  arm, no reflection driven by it.
- **Two surfaces exist only on booklets.** `plan_booklet` builds `Front`+`Back` and pairs them;
  `plan_nup` and `plan_step_repeat` both terminate in `one_surface_sheet(...)` → a single `Front`
  surface. `NUp` and `StepRepeat` carry **no duplex field**.
- **The booklet "flip" is duplex registration, not a work style.** `flip = side == Back &&
  short_edge` feeds `place_best(..., flip)` (long-edge: no flip; short-edge: 180°, per
  `m1-design.md`). Orthogonal to work style.
- **Marks already derive per-surface from cells in sheet space.** `attach_marks` →
  `plan_surface_marks` frames marks off each surface's placed trims/bleeds; `marks::reflect_about_x`
  already exists (booklet half-leaves use it). **Reflecting back-surface cell positions makes the
  back marks follow for free.**
- Backend emits **one PDF page per `Surface`** (`qpdf_backend.rs`), marks appended in sheet space.

**Nuance that sets the priority:** for a symmetric 2-up saddle booklet, sheetwise and work-and-turn
yield the *same plate*, and the current booklet output already registers under either. So on
booklets, work style is mostly pressroom **metadata** — the genuinely new behaviour is **gang / N-up
duplex**, which doesn't exist yet. The "prerequisite" (a duplex back surface) *is* the feature.

## Transform model (SPEC §9)

Each work style is a **sheet-space position transform** `T` mapping the front layout to the back
layout. SPEC §9's cardinal rule: it reflects *positions and marks*, **never page content** — no
negative-x scale on artwork.

| Work style       | Position transform `T`                        | Content orientation | Gripper        |
|------------------|-----------------------------------------------|---------------------|----------------|
| Sheetwise        | back imposed independently (own plate)        | upright             | same edge      |
| Work-and-turn    | reflect about **vertical** centerline `x↦W−x` | upright             | same edge      |
| Work-and-tumble  | reflect about **horizontal** centerline `y↦H−y` | upright           | moves to tail  |
| Perfector        | 180° rotation about sheet centre              | rotated 180°        | opposite edge  |

**Implementation primitive:** *reflect the target rect, then re-place the page upright into it* — not
"multiply the CTM by a mirror matrix" (that mirrors artwork). A helper
`back_placement(front_cell, work_style, sheet) -> (target_rect, orientation)` routes through the
existing `place_best` (the reflected `Rect` normalizes its own corners, so it conveys no orientation;
uprightness is guaranteed by `place_best` with `flip180=false`). Perfector is the only style that
legitimately rotates *content* (180° is a rotation, det +1 — allowed; a mirror is det −1 — forbidden).

**Sheetwise has no `T`.** It imposes the back **independently** from its own back source — it is *not*
a reflection of the front and must not be routed through the reflection path. `back_placement` returns
the back's own independent grid for `Sheetwise` and the reflected rects for the flip styles.

**Gripper edge & the cell grid.** Reflecting *placed* front cells (not re-running `grid_cell_rect`)
makes the empty gripper band reflect to the correct edge automatically, so the back **cell grid**
self-corrects for tumble/perfector — the grid geometry needs no gripper-edge parameter. What does
*not* self-correct: (1) sheet-edge **furniture** (slug/colour-bar/barcode), which is pinned to the
front gripper edge (see Marks); (2) **representability** — `Sheet` cannot yet express *which* edge
holds the gripper, so a `work-and-tumble` job with the default bottom gripper is silently
inconsistent. Both are why **tumble + perfector are deferred to Phase 2** behind a gripper-edge enum;
**Phase 1 ships only the gripper-preserving styles (sheetwise + work-and-turn).**

## Core invariant

Every cell **content CTM keeps det > 0** (180° included). Reflections live in *placement position*,
never in the content matrix. A single debug assertion + test enforces the whole §9 "never mirror the
artwork" rule.

## Data-model changes

- `NUp` / `StepRepeat` gain an optional duplex back: `back: Option<BackSpec>`.
  **Back-source model (decided — was Risk 2):** `BackSpec { source: String }` names a **second source
  id** already declared in `job.sources` (not a magic flag on the front source). The back art is real
  artwork, never a content mirror of the front.
- **Pairing rule (decided):** back cell `(r,c)` sits behind front cell `(r,c)` and draws a back-source
  page chosen by the **same fill order** as the front grid.
  - *Step & Repeat:* the front gang repeats one design, so the back is one back design repeated into
    every reflected cell (the common card case). `BackSpec.source` page 1 unless a page is named.
  - *N-up:* back page index = front page index (1:1 by fill order), including the partial last sheet.
- **v1 constraint — equal trim geometry.** Each back page's **TrimBox must match its paired front
  page's TrimBox** (size; bleed within tolerance). Otherwise `place_best` would centre the back at a
  different scale/position and the cut line would not register — so a mismatch is a hard
  `EngineError`, not a silent best-effort. (Differing back sizes / anamorphic backs are future work.)
  Equal trim ⇒ back scale, bleed gutter, and inner-bleed creep are identical to the front by
  construction, so the front-derived clip/creep is correct for the back; **no separate re-derivation
  needed in v1.** A `back_count != front_count` mismatch is likewise an `EngineError`.
- `one_surface_sheet` → conditional `two_surface_sheet(front_cells, back_cells)`; for the flip styles
  the back cells are the front cells run through `T` with the back art placed upright into the
  reflected rects; for `Sheetwise` the back is its own independent grid.
- **`rotate_to_fit` must be resolved once.** `place_best`'s `rotate_to_fit` picks +90° per call by
  area; on a duplex back it could rotate the back independently of the front and desync registration.
  The resolved front rotation is reused verbatim for the back (equal trim ⇒ same decision anyway; the
  reuse is belt-and-suspenders and is asserted by test).
- **Gripper edge (Phase 2).** `Sheet` has a scalar `gripper_pt` but no gripper *edge*. Phase 2 adds a
  `GripperEdge` enum (default `Bottom`, back-compat) so tumble/perfector can reserve the moved gripper
  and place furniture relative to it, and so the planner can **reject** an inconsistent
  work-style/gripper combination at plan time. Phase 1 needs none of this (its two styles keep the
  gripper on the same edge).

## Marks

Two families, two behaviours:

- **Cell-derived marks** (crop, registration, centre, trim outline) frame each surface's own placed
  cells in sheet space (`plan_surface_marks` builds the extent from `input.cells`). These **follow
  the back cells for free** — gang crop marks land mirrored, correct. Because Phase 1's layout is a
  *centred* block and `T` is a reflection about the sheet centreline, the back content extent maps
  onto the front extent, so **front and back registration coincide at the same sheet datum** — which
  is exactly what "punch through" requires. (The earlier "back target at `T(x,y)` per point" framing
  was imprecise: registration is emitted once on the union extent, not per target.)
- **Sheet-edge furniture** (slug, colour bar, barcode) is *not* cell-derived — `region_origin` /
  `slug_text` anchor it relative to the surface's **gripper edge**. `SurfaceMarkInput.gripper_edge`
  (`Bottom`/`Top`) carries that per surface: front (and sheetwise / work-and-turn backs) keep `Bottom`;
  tumble / perfector move the back's gripper to the tail, so its furniture is computed in the canonical
  gripper-at-bottom frame and then **reflected to the top edge** (`reflect_furniture_to_top`: rects
  mirrored about the sheet centre, slug text origin `y ↦ h − y − size` with glyphs kept upright). So
  furniture never lands in the gripper bite, on any style. *(Phase 2b — done.)*

## Determinism

Reflections and the 180° rotation are exact arithmetic routed through `geom::fmt`. Reuse the
`marks_output_is_byte_deterministic` pattern for two-surface gang output.

## Test matrix vs Quite Imposing

- **Pure (no qpdf, fast):**
  - **Positional reflection** is the real oracle for the flip styles: assert the back cell's
    sheet-space trim equals `T(front trim)` (e.g. work-and-turn: `back.trim == reflect_x(front.trim,
    W/2)`). *Not* the CTM x-scale sign — every style places content upright so x-scale is always
    positive (and is `0` for 90°-rotated pages), so that assertion neither discriminates nor is
    well-defined.
  - **`det > 0`** content-CTM invariant across all cells (the §9 no-mirror rule; det stays +1 under
    90°/180° rotation, so it is well-defined for rotated input).
  - **Involution** `T∘T = identity` for the flip styles — a sanity check on `T` (positions only), not
    the no-mirror rule.
  - **Sheetwise back** uses the *same independent grid* as the front (asserted positively, so the
    absence of a reflection is checked, not merely implied by omission from the involution list).
  - **Source `/Rotate` on a back:** a back page with `/Rotate=90` placed via `T` keeps
    `placed orientation == rotate_cw(source_rotate) ∘ T` and `det > 0`.
  - **`rotate_to_fit` resolved once:** front and back pick the same rotation.
  - **Mismatch errors:** unequal back/front trim size, and `back_count != front_count`, both
    `EngineError`.
- **QI parity (Phase 1, coordinate-level):** export a 2-up and a 3×N gang work-and-turn from QI;
  compare back-surface cell positions to `T(front positions)`.
- **Backend (Phase 2):** a two-surface gang renders 2 PDF pages, cell-derived back marks mirror
  front, gs cross-check clean, byte-deterministic. New `manual-tests/*.json` reference jobs (mirroring
  the existing testcard / metamorphosis QI references).

## Phasing (lands green per phase)

Re-scoped after adversarial review: tumble/perfector need the gripper-edge model (furniture + plate
representability), so they move out of Phase 1.

- **Phase 1 (done) — gripper-preserving styles, pure transforms.** `Sheetwise` + `WorkAndTurn`;
  `back: Option<BackSpec>` (second-source-id) on `NUp`/`StepRepeat`; pairing + equal-trim-geometry
  validation; `back_placement` (reflection for W&T, independent grid for sheetwise) →
  `duplex_sheet`; `rotate_to_fit` resolved once. Pure tests: positional reflection, `det > 0`,
  involution, sheetwise-independent-back, `/Rotate`-on-back, mismatch errors.
- **Phase 2a (done) — tumble + perfector position transforms.** `WorkAndTumble` (reflect about the
  horizontal centreline) and `Perfector` (180° about the sheet centre, content rotated 180° via
  `place_best(flip180)`, still `det > 0`). Cell-derived marks (crop/centre/trim/registration) reflect
  with the cells. **Sheet-edge furniture** (slug/colour-bar/barcode) on a gripper-moving style is
  **rejected** (`FurnitureOnMovedGripper`) — no silent furniture in the gripper bite. Pure tests cover
  both transforms + the guard; all four styles verified end-to-end through the CLI (`manual-tests`).
- **Phase 2b (done) — gripper-edge furniture model.** `GripperEdge {Bottom,Top}` on
  `SurfaceMarkInput`; `attach_marks` derives each surface's edge (back of a gang/N-up tumble/perfector
  job → `Top`); furniture is reflected to that edge (`reflect_furniture_to_top`), glyphs upright. The
  2a `FurnitureOnMovedGripper` guard is **lifted** — all four styles take any mark set. (A
  *user-configurable* sheet gripper edge + `grid_cell_rect` generalisation to Left/Right is future, not
  needed by work styles, which only move bottom↔top.)
- **Phase 2c — backend QI parity + reference jobs.** Two-surface byte/CTM parity vs Quite Imposing;
  `manual-tests` reference jobs with real distinct front/back art.
- **Phase 3 — booklet work_style metadata + leftovers.** Make `work_style` meaningful on the booklet
  paths (mostly metadata, since a symmetric 2-up turn ≡ sheetwise plate); any deferred furniture/colour.

## Risks / open decisions

1. **Duplex vs WorkStyle (resolved).** Orthogonal: `Duplex` (long/short edge) is the booklet back
   *registration flip*; `WorkStyle` is the *plate-sharing / position* strategy for the gang/N-up back.
   They never double-apply — booklets don't take a `BackSpec`; gang/N-up don't take `Duplex`.
2. **Back-artwork source model (resolved).** Second source id (`BackSpec { source }`), paired 1:1 by
   fill order, **equal trim geometry required in v1** (mismatch → `EngineError`). Never a content
   mirror. Differing back sizes / anamorphic backs are future work. (See Data-model changes.)
3. **Gripper edge (Phase 2).** Scalar `gripper_pt` → add `GripperEdge` enum; only tumble/perfector
   need it, so it gates them. Phase 1's two styles keep the gripper on the same edge.
