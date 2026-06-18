# M1 Design — useful single-sheet & bound imposition

> Status: Draft · 2026-06-18 · Internal (non-user-facing). Extends `SPEC.md` §10/§16.
> Built TDD: the pure planner/geometry/ordering is implemented + tested this milestone; PDF
> *emission* of marks/bleed/slug is specified here and covered by `#[ignore]`d executable tests.

## Scope (from product owner notes)

1. **Printer marks** — a broad, individually-toggleable, customizable mark system (taxonomy below).
2. **Slug** (info area) + **Flip** (extra fold-over allowance for the topmost signature of a hardcover).
3. **Step & Repeat** — customizable spacing, bleed / no-bleed modes.
4. **Saddle Stitch** *and* **Perfect Bind** (perfect bind = several saddle-stitch micro-booklets / signatures, gathered).
5. **Surfaces kept for custom placement** (manual scheme); autocutter marks relevant here.
6. TDD — exhaustive tests, including not-yet-passing ones (kept as `#[ignore]` specs).

## Marks taxonomy (each individually toggleable; `MarkSet` in `vgdi-types::marks`)

**Cut / trim**
- Corner crop marks — styles: **Classic** (offset corner ticks), **Full-line** (lines spanning the sheet), **Japanese/double** (inner trim + outer bleed pair). Configurable length / offset (≥ bleed) / weight / color.
- Center marks (midpoint of each edge).
- Trim-box outline.

**Registration / alignment**
- Registration targets (bullseye) — authored in **Separation colorant `All`** (every plate), positions: corners / edge-centers / all.
- Star targets (slur/dot-gain).
- Side-lay / front-lay (gripper-edge) marks.
- Punch / pin-register marks.
- Camera/vision marks (die-cut/convert).

**Fold / bind**
- Fold marks (dashed).
- Spine / binding marks.
- Collation / gathering **back-step** marks (perfect-bind signature sequencing).
- Score / perforation marks.

**Colour / quality control**
- Colour bar / control strip — process CMYK solids, tint ramps, spot patches.
- Densitometer / gray-balance patches.
- Dot-gain / slur / doubling targets.
- Ink-key zones (CIP3-aligned).

**Cutting automation (autocutter)**
- Guillotine programming barcode (e.g. POLAR / Wohlenberg).
- Cut marks / cut registration for automatic cutters.
- OMR marks.
- Cutter lay / edge marks.

**Information / slug**
- Sheet info: filename, date/time, sheet #, surface (front/back), separation/plate, operator, job #, colour. (Token-driven; user supplies any literal text.)
- Job barcode / QR.
- Per-cell page-number labels.
- Station numbers (step & repeat).

**Other**
- Bleed-area treatment: none / outline / **hatched**.
- Gripper-margin indicator.
- Bearer bars (packaging).
- Identification / micro dots.

Customization axes shared by most: line **weight**, **length**, **offset** (crop offset must be ≥ bleed),
**color** (`RegistrationAll` | process tint | named spot), on/off.

## Schemes (`vgdi-types::scheme::Scheme`)

- `NUp { rows, cols, fill, scale, gutter, rotate_to_fit }`
- `StepRepeat { rows, cols, h_space, v_space, bleed_mode: Bleed|NoBleed }`
- `SaddleStitch { duplex, cover, scale }` → 2-up printer spreads, front/back **Surfaces**.
- `PerfectBound { signature_pages, duplex, scale }` → split into saddle-stitch signatures, gathered.
- `Manual { surfaces: [ManualSurface{ placements:[Placement] }] }` → explicit per-surface placement (custom + autocutter).

## Binding / signature model

- **Pad** page count up to a multiple of 4 (saddle) / signature size (perfect).
- **Saddle stitch** ordering for P pages (1-based), per sheet-side 2-up: outer sheet = `(P, 1)` front, `(2, P-1)` back, … working inward. Implemented in `engine::imposition::saddle_order`.
- **Perfect bind**: split into ⌈P / sig⌉ signatures of `signature_pages` each, each saddle-ordered, gathered in order. Collation back-step marks identify signature order.
- **Surface** = one side (front/back) of a sheet holding placed cells; the back-side transform is derived from `duplex` (long-edge: no flip; short-edge: 180°). Zero-creep in M1 (creep is v1).
- **Flip**: extra fold-over allowance added to the binding edge of the **topmost** signature (hardcover wrap); modeled as an extra margin on that signature's outer surface.

## Plan model change

`PlannedSheet { width, height, surfaces: Vec<Surface> }`, `Surface { side: Front|Back, cells: Vec<Cell>, marks: MarkPlan }`.
N-up / step-repeat = one `Front` surface per sheet; saddle/perfect = `Front`+`Back`. The qpdf backend
emits **one PDF page per surface**, so all schemes render through the existing `Do` placement path.

## What lands this milestone (green) vs deferred (`#[ignore]` specs)

- **Green now:** type model; saddle + perfect ordering; step & repeat; placement (fit/none/fixed + rotate-to-fit);
  surfaces; mark **geometry** computation; schemes render to PDF via existing placement.
- **Deferred (specs written, ignored):** PDF **emission** of marks (incl. Separation `All`), bleed-pull clip,
  slug text, colour bars, barcodes/autocutter, hatched bleed, Flip allowance rendering.
