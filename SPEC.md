# very-good-document-imposer — Technical Specification

> Status: **Draft 0.2** · 2026-06-18 · Internal engineering spec (non-user-facing).
> (0.2 = post-adversarial-review corrections: qpdf binding reality, isolated-wrapper
> transparency rule, page-box fallback, X-1a boxes, determinism scoping, NOTICE compliance,
> M0/Spike-0 roadmap split.)
> This document specifies architecture, scope, and the prepress-correctness contract.
> It deliberately contains **no user-facing copy** — all UI text, CLI help text, and
> error message wording are left as explicit gaps marked with the literal token `∑CG`
> (grep for it) to be authored separately.

---

## 1. Overview & Product Vision

A robust, cross-platform, **standalone native** PDF imposition application — a serious
alternative to Quite Imposing, Montax Imposer, Imposition Studio, Imposition Wizard,
and (at the high end) Kodak Preps. It bridges two worlds that existing tools split:
simple booklet/N-up jobs and pro signature/press-sheet planning.

**Hard product constraints (locked):**

| Constraint | Decision |
|---|---|
| Form factor | Standalone app. **Not** an Acrobat/InDesign plug-in. |
| Platforms | macOS, Windows, **and** Linux. |
| UI | Native-feeling. **No Electron, no webview (Tauri excluded).** |
| Licensing | **Maximally permissive ("0BSD-maxxing")** — see §3. |
| First deliverable | **Headless engine + CLI first; GUI is a later phase.** |
| Target user | **Professional prepress / print shops** (strict fidelity bar). |
| Input | **PDF only** for v1 (PDF/X-centric). No PostScript/EPS ingest. |

**Positioning vs the field:** the live WYSIWYG sheet canvas (eventual GUI) is the
differentiator vs Quite Imposing's dialog/wizard paradigm; the determinism + permissive
licensing + native multiplatform reach is the differentiator vs the closed,
Acrobat-bound or single-OS incumbents. We are **not** chasing MIS/estimating
integration or cost-based gang optimization (Metrix/Signa territory).

---

## 2. Goals, Non-Goals & Target Users

### Primary user
Professional prepress operators and small/mid print shops. They expect: TrimBox-anchored
placement, CMYK/spot/ICC/overprint preservation, correct printer marks, creep/shingling,
work-and-turn/tumble, PDF/X preservation. RGB-only "preview" is acceptable **only if
honestly labeled as not a proof** (see §3 soft-proof gap).

### Secondary user (same engine, progressive-disclosure UI later)
Office / light users doing booklets and simple N-up. Served by a *subset* of the same
UI — **we do not build a second product**.

### Non-Goals
- Acrobat/InDesign plug-in form.
- Electron / bundled-Chromium / webview UI.
- PDF content authoring/editing (no text editing, no design tooling — imposition only).
- Becoming a RIP or a color-management authoring suite (we *preserve* color; we don't build a RIP).
- Enterprise MIS/estimating integration; cost-based gang optimization at the Metrix/Signa level.
- Rasterization as the placement mechanism (we **never** rasterize to impose; raster is preview/soft-proof only).
- Non-PDF input formats in early versions.

---

## 3. Licensing & Dependency Policy

**Project license target: 0BSD** (or equivalently permissive) on all first-party code.

### Allowed dependency licenses
`0BSD`, `MIT`, `BSD-2/3-Clause`, `Apache-2.0`, `zlib`, `ISC`, `Unlicense`.
`MPL-2.0` permitted (file-level copyleft — triggered only by distributing *modified* MPL-covered
files in source form; static-linking an unmodified MPL crate into a 0BSD binary requires no
disclosure of our code, only source availability + preserved notices for the MPL files). Prefer
to avoid for simplicity.

### Forbidden in the shipped product
**`GPL-*` and `AGPL-*`.** This rules out **MuPDF, Ghostscript, Poppler, iText**, and the GPL
**`podofoimpose`** tool (the PoDoFo *library* is fine; only its bundled `podofoimpose` CLI is
GPL-2.0 and must not be copied into our source — reference only). Note: **qpdf ships no GPL
tool** — both the qpdf library and its CLI are Apache-2.0.

### LGPL
Acceptable **only** for an optional, **dynamically-linked** dependency with a preserved
relink path — realistically this only ever applies to a future GUI toolkit (Qt LGPL),
never to the engine. The engine and CLI stay 100% permissive and statically linkable.

### Approved core dependencies (engine + CLI)

| Purpose | Library | License | Notes |
|---|---|---|---|
| Page→Form-XObject placement (the core primitive) | **qpdf ≥ 7** | Apache-2.0 (pre-7 was Artistic-2.0) | Verbatim foreign-object copy preserves CMYK/spot/ICC/overprint and *copies* an existing page `/Group` when present. **Caveat:** the page→form-XObject helper (`QPDFPageObjectHelper::getFormXObjectForPage`) is **C++-only** — absent from `qpdf-c.h` and the `qpdf` Rust crate; we bind it first-party (see ADR-0001 §2). Build via the crate's `vendored` feature (compiles zlib/jpeg/**native** crypto via `cc`); **must force native crypto and disable GnuTLS/OpenSSL** auto-link (GnuTLS is LGPL — would taint the permissive graph). |
| Object model / low-level read (**research spike**, not a drop-in fallback) | **lopdf** (Rust) | MIT | Low-level parse/object model only — **no** page→form-XObject helper, resource-collision merge, or overprint/group/box logic. Choosing it = a from-scratch reimplementation of the placement primitive (multi-week), required to pass the *same* golden suite as the qpdf backend. |
| Marks/furniture authoring | direct content-stream emit, or **pdf-writer** | MIT/Apache-2.0 | `pdf-writer`/`krilla` **cannot embed external pages** — use only to author sheet-level vector marks, never for source placement. |
| Color math (when needed) | **lcms2 / Little-CMS** | MIT | ICC transforms; we preserve, not convert, by default. |
| Preview rasterization (**GUI phase only**) | **PDFium** via `pdfium-render` | BSD-3 + Apache-2.0 + permissive bundled set (FreeType/FTL, HarfBuzz, libtiff, OpenJPEG, Abseil, libpng, zlib, ICU, Noto/OFL-1.1) | RGB/BGRA only. Not needed for engine/CLI v1. GUI build must ship a consolidated NOTICE (§3.1). |

### Known permissive ceiling — the soft-proof gap (must be honest about this)
**No permissive renderer does accurate CMYK/overprint/spot soft-proofing.** Only
MuPDF/Ghostscript do, and both are AGPL-or-commercial → excluded by our license policy.
Consequences:
- Engine/CLI v1 needs **no** renderer at all (placement is vector; output is a PDF).
- The future GUI ships an **honest RGB preview**, explicitly *not* a contract proof.
- Accurate soft-proof is a **paid/out-of-policy** feature deferred indefinitely; revisit
  only if the licensing stance changes.

### 3.1 Attribution & NOTICE compliance
0BSD on *our* code waives downstream attribution, but it does **not** discharge the inbound
obligations we incur by statically linking + redistributing permissive deps: Apache-2.0 §4
NOTICE propagation (qpdf; later Abseil), BSD binary-notice (later PDFium/OpenJPEG), MIT "all
copies" (lopdf, lcms2), FreeType FTL acknowledgment, and the RSA-MD5 notice (qpdf native
crypto). Requirement: generate a `THIRD-PARTY-NOTICES` file (`cargo-about`/`cargo-deny` over the
Rust graph + a hand-maintained section for the qpdf vendored bundle: zlib, libjpeg, native
crypto; and, at GUI phase, PDFium's bundle), ship it alongside the CLI and GUI, and surface it
via a `--license` output. `cargo-deny` enforces the allowed-license set (§3) in CI.

Distinguish **qpdf-embedded** deps (native crypto: public-domain Rijndael + MIT-style SHA2 +
RSA-notice MD5 — the last carrying a mandatory acknowledgment clause) from **external build
deps** qpdf links but does not vendor (zlib, libjpeg/-turbo). All permissive; all go in the
NOTICE inventory.

---

## 4. Glossary

- **Imposition** — arranging multiple finished pages onto a larger press sheet so that, after printing, folding, and cutting, pages end up in the right order/position.
- **Signature** — a folded press sheet forming a section of a book.
- **Press sheet / Surface** — the physical sheet; Surface = one side (front/back).
- **Work style** — sheetwise, perfector, **work-and-turn**, **work-and-tumble**, come-and-go: how front/back are printed and the sheet is flipped.
- **Creep / shingling** — progressive per-sheet shift compensating for paper thickness in folded/nested signatures.
- **Bottling** — page rotation compensation for fold skew (later).
- **Gripper** — the non-imageable press margin where the sheet is held.
- **N-up / Step-and-repeat / Cut-and-stack / Booklet** — the core imposition schemes (§10).
- **Page boxes** — MediaBox, CropBox, TrimBox, BleedBox, ArtBox (§8).
- **Form XObject** — a reusable PDF content object; placing a source page as a transformed Form XObject is *the* imposition primitive.
- **OutputIntent** — the ICC characterization (e.g. Fogra/GRACoL) a PDF/X file targets.
- **Marks / furniture** — crop, fold, registration marks, color bars, slug/job info.

---

## 5. Architecture: Headless Engine + CLI + (later) Thin Native UI

Three layers joined by **coarse, data-only contracts**:

```
            ┌──────────────────────────────────────────────┐
            │  (LATER) Native GUI shell — thin              │
            │  edits JobSpec · blits preview bitmaps        │
            └───────────────▲──────────────────────────────┘
                            │  JSON JobSpec  /  bitmap tiles
            ┌───────────────┴──────────────────────────────┐
   CLI ─────►  Stable contract surface                      │
 (v1 deliverable) a) Rust library API                       │
            │   b) C ABI (cdylib): submit_job / render_tile │
            │   c) CLI binary: `impose job.json -> out.pdf` │
            └───────────────▲──────────────────────────────┘
                            │
            ┌───────────────┴──────────────────────────────┐
            │  ENGINE (pure Rust, no UI deps)               │
            │  parse → plan (ImpositionPlan) → emit PDF     │
            │  verbatim Form-XObject placement (qpdf)       │
            │  marks/bleed authoring · color preserve       │
            │  DETERMINISTIC: same JobSpec ⇒ byte-stable PDF│
            └───────────────────────────────────────────────┘
```

**Why this shape:** the engine is the entire product's value (the geometry/fidelity math),
so it must be independently testable, scriptable, and fuzzable. The CLI **is** the
automation contract (hot-folder/CI/render-farm). When the GUI arrives it is *just another
client* of the same boundary — no QObject or live PDF object ever crosses FFI. This also
lets the eventual UI toolkit decision stay deferred and orthogonal.

**Engine language: Rust** (rationale in ADR-0001). Memory safety converts geometry bugs
(CTM, `/Rotate`, TrimBox anchoring, creep, blend-space) into `Result`s rather than silent
mis-placement UB — uniquely valuable when "the product *is* the geometry."

---

## 6. Technology Stack & Rationale

See **`docs/adr/0001-platform-and-stack.md`** for the full decision record, scorecards, and
rejected alternatives. Summary:

- **Engine:** Rust, fully permissive toolchain. **qpdf (Apache-2.0)** for verbatim
  page→Form-XObject placement is the load-bearing choice (the one library purpose-built for
  lossless content+resource copy with a transform matrix). Placement sits behind an
  `ImpositionWriter` trait so a pure-Rust `lopdf` backend can replace it later without
  touching the planner.
- **CLI:** Rust binary over the engine; JSON/TOML `JobSpec` in, PDF out.
- **GUI (later phase):** deferred. Constraints: native-feeling, permissive toolkit. Slint is
  GPL/commercial → **excluded** by §3. Candidate set when the phase opens: Qt Widgets
  (LGPL, dynamic-link) for the best native canvas (QGraphicsView), or a permissive
  alternative (wxWidgets / Avalonia-MIT / egui) — decided then, not now.

---

## 7. Core Domain Data Model

The engine is pure functions over an explicit, serializable `JobSpec`. Entities:

- **JobSpec** *(root)* — single source of truth the CLI/UI edit. References `Source`s, a
  `Scheme`, one or more `Sheet`s, a `MarkSet`, a `ColorPolicy`, and an `OutputTarget`.
  Same JobSpec ⇒ byte-stable output.
- **Source** — a referenced input PDF + parsed per-page box metadata, `/Rotate`,
  page-group transparency color space, OutputIntent, PDF/X conformance. Immutable view.
- **SourcePage** — one page: its five boxes, `/Rotate`, **effective trim** (TrimBox → ArtBox;
  **reject if both absent** — never MediaBox/CropBox), and transparency-group attributes
  (`/CS`, isolated `/I`, knockout `/K`).
- **Scheme / LayoutSpec** — the rule: `NUp | StepRepeat | Booklet | CutStack | PerfectBound | Manual`, with parameters (grid rows/cols, fill order, repeat policy, signature length, work style).
- **Signature** — a planned folded section: sheet count, page→cell mapping, fold pattern, lay direction. Unit of creep/shingling and gather ordering.
- **Sheet (PressSheet)** — output sheet: size, orientation, gripper margin (non-imageable), up-count, work style, front/back surfaces.
- **Surface** — one side of a Sheet; holds placed Cells; back-side transform derived from work style + gripper/flip axis.
- **Cell / Slot** — a position on a Surface receiving one SourcePage as a Form XObject: CTM (scale/rotate/translate **anchored on TrimBox**), clip/bleed-pull region, creep offset.
- **Placement (PlacedPage)** — realized Cell+SourcePage binding: per-cell transform matrix + bleed clip + transparency-group wrapper carrying the page's blend color space.
- **MarkSet / Mark** — sheet-level vector furniture generated by the engine **between BleedBox and MediaBox**: crop/trim (offset ≥ bleed), fold (dashed), registration (in `[Registration]`/All colorant), color bars/control strips, slug/job-info (variable tokens).
- **ColorPolicy / OutputIntent** — pass-through CMYK/spot/DeviceN/ICC rules; overprint preservation (`OP`/`op`/`OPM`); single validated OutputIntent across merged sources; marks-color authoring rules.
- **ImpositionPlan** — the deterministic computed result (ordered Sheets/Surfaces/Cells with resolved geometry). The intermediate the UI previews and the writer serializes. Decouples *compute layout* from *emit PDF*.
- **Template** — a saved parametric JobSpec skeleton (Sheet + Scheme + MarkSet + ColorPolicy, minus Sources) for reuse/search/auto-match.
- **OutputTarget** — PDF version, PDF/X conformance (X-1a flatten vs X-4 live transparency), optional JDF/CIP3 metadata derived from the same geometry that drew the marks.

---

## 8. Prepress Correctness Model (the contract)

These are **hard invariants** enforced by the golden-file test suite (§13). Severity from
the prepress research.

### Critical invariants
1. **TrimBox anchor (one model, stated once).** Placement anchors on **TrimBox**; if absent,
   **ArtBox**. If **both** are absent the source is non-conformant for pro prepress and is
   **rejected/flagged** — the engine does **not** silently fall through to CropBox/MediaBox
   (that would shift every page). A permissive best-effort Crop→Media fallback may exist *only*
   behind an explicit opt-in flag, with output labeled non-fidelity. Enforce containment
   `Media ⊇ Bleed ⊇ Trim/Art`; reject violators rather than misplacing.
2. **`/Rotate` + per-page variance.** Normalize each page by folding `/Rotate` (0/90/180/270)
   into the CTM *before* placement; read boxes per page (they may differ); flag GWG
   "equal effective TrimBox across pages" deviations.
3. **Vector Form-XObject placement.** Wrap each source page (content stream + Resources) as
   a Form XObject placed via `cm`. **Never** rasterize or pre-flatten by default. Keep
   transparency live; target PDF/X-4. Flatten only when the OutputTarget forbids live
   transparency.
4. **Transparency-group blend space.** When compositing multiple pages onto one sheet, the
   engine **synthesizes an isolated wrapper** transparency-group XObject around each placed page
   (`/I true`) carrying the resolved blend `/CS` — the page-group color space, or the
   OutputIntent/device CMYK space if the source omits it. Per ISO 32000 §11.6.6, a group with an
   explicit `/CS` **must** be isolated, so the wrapper's `/I` is always true (**not** "preserved");
   the source page's *inner* group keeps its own `/I`/`/K` untouched inside the wrapper. qpdf does
   **not** synthesize this (it only copies an existing `/Group`) — wrapper synthesis is engine
   work. Isolation is exactly what firewalls one page's blend from a neighbor.
5. **Color preservation.** Pass through DeviceN/Separation colorants and named spots
   untouched; preserve `OP`/`op`/`OPM`; keep source ICC; carry exactly one compatible
   OutputIntent; **never auto-convert** color during imposition.

### Important invariants
6. **Marks placement.** Sheet-level, between BleedBox and MediaBox, correct colorants.
   Crop marks offset outward ≥ bleed; registration in `[Registration]`/All (so they hit
   every separation — *not* rich black); de-duplicate shared cut lines for ganged work.
7. **Bleed & gutter math.** Pull bleed only from each page's BleedBox band; require gutter
   ≥ 2× bleed between bleeding neighbors (or shared common bleed); distinguish **binding
   gutter** (spine) from **trim gutter** (cut). Flag/clamp insufficient bleed — never invent
   pixels (edge-mirror only on explicit opt-in).
8. **Creep / shingling.** Per-signature progressive shift driven by paper caliper + total
   page count, growing outer→inner; crossover/spread-aware so images align across the spine.
   *Validate direction/magnitude against Preps/Kodak references and physical proofs.*
9. **Work styles.** Model gripper edge + flip axis explicitly: **work-and-turn** reflects the
   *cell-layout positions and marks* across the **vertical** centerline (one plate set, constant
   gripper); **work-and-tumble** reflects across the **horizontal** centerline (gripper moves to
   the tail edge); **perfector** = back surface is the front rotated **180°** with gripper on the
   opposite edge. The reflection is a *position* reflection — each placed page keeps normal,
   un-mirrored orientation (**never** apply a negative-x scale to page content). Back-side
   transform must match the work style. Reserve gripper as non-imageable.
10. **OutputIntent merge.** Verify all sources share a compatible OutputIntent; refuse/warn
    on conflict; emit exactly one valid OutputIntent.

### Nice / deferred
11. **Minimal-region rasterization** only when flattening is genuinely unavoidable
    (e.g. PDF/X-1a output, irreproducible blend+overprint+spot): clip and rasterize *only*
    the complex overlap region at resolution tied to final scale (≥300–600 ppi contone,
    higher for line art), composite in the correct device space, keep everything else vector.
12. **JDF / CIP3 (PPF)** cut/fold metadata derived from the same geometry that drew the marks.

---

## 9. PDF Standards & Conformance

- **PDF/X-1a** (ISO 15930-1/-4): CMYK+spot only, **no** live transparency (pre-flattened),
  no live RGB/ICC; mandates **MediaBox + (TrimBox or ArtBox) + a `GTS_PDFX` OutputIntent**;
  **BleedBox only when content bleeds** (a no-bleed job is conformant without it). Imposing X-1a
  is geometrically simpler (already flat); preserve device CMYK + spots, don't reintroduce
  transparency/RGB.
- **PDF/X-4** (ISO 15930-7): **target output** by default. Allows live transparency,
  ICC-managed color, spot, optional OCG layers; requires Media + (Trim **or** Art),
  `Trim/Art ⊆ Bleed ⊆ Media`, OutputIntent. Keep transparency live to the RIP (APPE model).
- **OutputIntent:** one valid intent on output; verify source compatibility before merge.
- **GWG (GWG2015/2022):** validate bleed present (≈3 mm), TrimBox present & equal across
  pages, no CropBox for trim.
- **CIP3 PPF / CIP4 JDF:** later-phase production metadata; if emitted, geometry must match
  drawn marks.

---

## 10. Imposition Schemes & Algorithms

Each scheme is a pure planner: `(JobSpec) → ImpositionPlan`. Parameters per scheme:

- **N-up** — fixed grid; per-cell scale mode (fit/none in M0; auto-fit + rotate-to-fit in M1); row/column-major fill. *(M0)*
- **Step & Repeat** — uniform grid; repeat count + fill policy (repeat-from-start / repeat-last); optional rotate-to-fit. *(M1)*
- **Booklet / saddle-stitch** — 2-up printer spreads; short/long-edge duplex; cover handling; creep hook. *(M1; **zero-creep** — valid only where caliper × page-count makes creep negligible; creep added in v1)*
- **Cut & Stack** — single + multi-up; sequential numbering; partial-last-stack balancing. *(v1)*
- **Perfect-bound / multi-signature gather** — fixed + mixed signature lengths; spine/cover. *(later)*
- **Manual / assembly** — drag-place any page at any scale/position (needs GUI canvas). *(later)*
- **Gang-up** — manual placement then auto-fill optimization; mixed sizes/quantities. *(later)*

Work styles (sheetwise/perfector/work-and-turn/work-and-tumble) parameterize Booklet,
Cut&Stack, and signature schemes (§8.9).

---

## 11. UI / WYSIWYG Canvas Specification — *(deferred, later phase)*

Out of scope for engine/CLI v1. When the phase opens:
- A zoomable/pannable WYSIWYG sheet canvas (transform-matrix zoom/pan, hand-drag, hit-test,
  snapping/guides) showing the `ImpositionPlan` with live RGB preview tiles from the engine.
- Native menus/dialogs/print/file-pickers/drag-and-drop.
- Manual imposition + assembly mode; template save/load/search.
- The shell only mutates the `JobSpec` and blits engine-rendered bitmaps — it holds no
  imposition logic.
- **`∑CG`:** all UI strings are to be authored by the product owner, not drafted here.

---

## 12. CLI, Scripting & Automation Interfaces

**Tier 1 — CLI + JobSpec (v1).** `impose <job.(json|toml)> -o out.pdf`. The JobSpec schema is
the public contract; the CLI is fuzz/golden tested. Sketch:

```jsonc
{
  "schema": "vgdi/jobspec@1",
  "sources": [{ "id": "body", "path": "body.pdf" }],
  "scheme": {
    "kind": "booklet",
    "duplex": "long-edge",
    "creep": { "paper_caliper_pt": 0.30 }
  },
  "sheet": {
    "size_pt": [1190.55, 841.89],          // SRA3-ish, in points (1pt = 1/72")
    "work_style": "work-and-turn",
    "gripper_pt": 28.35
  },
  "marks": {
    "crop": { "offset_pt": 8.5, "length_pt": 14.2 },
    "registration": true,
    "color_bar": "process+spot",
    "slug": ["{filename}", "{sheet}", "{side}", "{date}"]   // tokens, not user copy
  },
  "color_policy": { "preserve_spots": true, "preserve_overprint": true },
  "output": { "pdfx": "X-4", "pdf_version": "1.6" }
}
```

**Tier 2 — embedded scripting (later).** Rhai/Lua job-*builders* that emit JobSpecs; scripts
never touch PDF internals.

**Tier 3 — JSON-RPC server / watch-folder mode (later).** Same engine, batch orchestration.

One tested core; many front ends.

---

## 13. Determinism & Testing Strategy

**Determinism is a hard invariant, scoped honestly.** For a **pinned, vendored qpdf SHA** + fixed
writer settings (deterministic `/ID` on, object-stream mode fixed, compression fixed, no
`/CreationDate`/`/ModDate`), **the same JobSpec produces a byte-identical PDF**. qpdf disclaims
*cross-version* reproducibility, so the vendored qpdf SHA is part of the determinism contract and
a golden-suite axis — regenerate goldens on qpdf bumps. No wall-clock, no locale, and no `HashMap`
iteration order leaks into output.

- **Golden / snapshot tests** over a corpus of **real CMYK / spot / overprint / transparency
  artwork** (model the discipline on krilla's visual-regression suite).
- **Fuzz** the JobSpec → plan → emit path and the CLI boundary.
- **Structural assertions** on output: page boxes, OutputIntent count, colorant survival,
  XObject (not raster) placement, marks emitted in a **Separation colour space with colorant
  name `All`** (tint transform → 100% of every process colorant; `[Registration]` is only an
  Adobe UI swatch alias, never the on-the-wire name), gripper clear.
- **Placement golden** pinning the exact CTM for a `/Rotate=90` page with a **non-zero-origin
  TrimBox** (the classic off-by-a-translation bug) — the engine owns this regardless of which
  qpdf interface is chosen.
- **Cross-check rendering** of golden outputs (dev-only, may use any renderer offline for
  *test verification* — not shipped) against expected pixels.
- **Containment/rejection tests:** malformed/out-of-spec sources are rejected, not misplaced.

---

## 14. Performance & Resource Model

- **Never rasterize to impose** → an N-up sheet is ≈ source size regardless of cell count.
- Rasterize only for preview (GUI phase), at preview DPI, with tiling + thumbnail cache.
- Stream output **sheet-by-sheet**; use xref/object streams; **dedupe shared fonts/images**
  across sheets; cache per-object ICC transforms.
- Parallelize the imposition/write path with `rayon` for gang jobs.
- **PDFium is not thread-safe** (GUI phase): one-document-per-worker (process or serialized
  thread) for preview; the pure-Rust placement/write path parallelizes freely.

---

## 15. Distribution, Signing & Auto-Update

Scaffold from day one (the engine/CLI ships too):
- **macOS:** `codesign` + `notarytool` + staple; Developer ID. Every embedded native blob
  (qpdf static lib avoids a separate signable; future PDFium dylib must be signed) handled in CI.
- **Windows:** Authenticode / Azure Trusted Signing.
- **Linux:** Flatpak + AppImage (and a plain tarball/CLI).
- **Auto-update:** later (GUI phase); CLI distributed via package managers + releases.

---

## 16. Phased Roadmap

### Spike-0 — feasibility gate (before any scheme code)
- A throwaway Rust program that **statically links qpdf** and places **one** source page as a
  transformed Form XObject, building **green in CI on macOS, Windows (MSVC), and Linux**. Proves
  the load-bearing binding/build thesis. **If Windows static-link fails, that is the trigger to
  evaluate the pure-Rust `lopdf` reimplementation** — before any scheme code exists.
- In parallel: author a **license-clean golden fixture corpus** (self-authored
  DeviceN/Separation/overprint/spot/transparency PDFs via a permissive writer + lcms2). The Ghent
  Output Suite may be *referenced* for manual spot-checks but **not vendored** (no stated
  redistribution license).

### M0 — walking skeleton (smallest end-to-end proof)
Single source PDF → **N-up only** (uniform fit/none scale; no auto-fit, no anamorphic) →
TrimBox-anchor + ArtBox fallback + **reject-on-absent** + `/Rotate` normalization folded into the
CTM + containment rejection → **verbatim Form-XObject placement** → per-page wrap as an
**isolated** transparency-group XObject carrying the resolved blend `/CS` (engine-synthesized) →
color pass-through (CMYK/spot/overprint untouched) → **byte-stable** PDF (pinned vendored qpdf SHA,
deterministic `/ID`, dates suppressed) → driven by the **CLI** consuming a JSON/TOML JobSpec.
**Excluded from M0:** marks, bleed-pull, booklet, Step & Repeat, anamorphic scaling, all
signing/notarization.

### M1 — useful single-sheet imposition
- **Step & Repeat**; **Booklet** (short/long-edge duplex, cover; **zero-creep**).
- Full page-box model completeness + per-page variance handling; N-up auto-fit + rotate-to-fit.
- Gutters/margins/binding-spine offsets, gripper reservation (binding vs trim gutter distinguished).
- Bleed handling (use BleedBox / define fixed / gutter ≥ 2× bleed rule / flag insufficient).
- Crop/trim/fold/registration marks in correct boxes + colorants (Separation `All`).
- Scale/rotate/mirror placed pages (**uniform** scale, 90/180/270, fit/fill; **anamorphic → v1**).
- Reproducible 3-OS CI build matrix + **unsigned** release artifacts (Linux tarball + `cargo install`).
  *(Paid code-signing/notarization deferred until a binary has a real recipient — see §18.)*

### v1 — Pro depth (still possibly headless-first; GUI may begin here)
- **Cut & Stack** (multi-up, numbering, partial-last-stack balancing).
- **Creep / shingling** (linear + custom curve, per-signature, crossover-aware, caliper-driven).
- **Work styles:** sheetwise/perfector/work-and-turn/work-and-tumble (gripper + flip-axis model).
- Press-sheet definitions + named stock library.
- Color bars/control strips; slug lines + variable tokens; page numbering/Bates; collation/gathering marks.
- Rule-based page shuffle/collation + assistant.
- Templates (save/load/parametric/search/auto-match).
- PDF/X-1a & X-4 preservation. (Per-page isolated-wrapper blend handling ships from M0; v1 adds
  the harder **PDF/X-1a flatten-time** blend/transparency handling.)
- **Anamorphic** (non-uniform x/y) scaling behind an explicit, warned opt-in.
- **GUI begins:** thin native shell + WYSIWYG canvas + manual imposition (toolkit chosen at this gate).

### Later
- Perfect-bound / multi-signature gather; folding-pattern library.
- Gang-up / ganging (manual → auto-fill, finishing/cost-aware).
- Bottling; web-growth/distortion compensation.
- Mixed page sizes (auto-rotate-to-fit, normalize).
- Automation sequences / hot-folders / Switch/AE integration.
- JDF / CIP3 (PPF) import + export.
- Variable-data merge (CSV, PDF/VT-aware).
- Tiling / page splitting for large-format.
- CMYK/overprint/spot soft-proof (**blocked by license policy** unless stance changes).
- Embedded scripting + JSON-RPC server mode.

---

## 17. Risks & Mitigations

| Risk | Mitigation |
|---|---|
| qpdf's page→form-XObject helper is **C++-only** (absent from `qpdf-c.h` and the pre-1.0, single-maintainer `qpdf` crate); and C++/zlib/jpeg static-link on **Windows MSVC** is the classic failure point. | **Spike-0 is the first gate** (§16): statically link qpdf + place one page on all 3 OSes before any scheme code. Bind the helper **first-party** (extend a `qpdf-sys` bindgen allowlist to `qpdfjob-c.h`, or re-implement form-XObject assembly on the C-API object model: `qpdf_oh_copy_foreign_object` + page content data + manual `/Group`,`/BBox`,`/Resources`,`/Matrix`). Hide behind `ImpositionWriter`. If Windows static-link fails → trigger the lopdf reimplementation evaluation. |
| No permissive renderer does CMYK/overprint soft-proof. | Engine/CLI need none. GUI ships honest RGB preview, never labeled a proof. Accurate proof deferred/out-of-policy. |
| Prepress correctness defects silently corrupt plates. | TrimBox-anchor + `/Rotate`-normalize + verbatim XObject copy as hard invariants enforced by a golden suite from real CMYK/spot/overprint artwork; reject non-conforming sources. |
| No off-the-shelf imposition in Rust (unlike C++ podofoimpose). | Use podofoimpose + Preps/Kodak fold docs as *references* (never copy GPL code); phase hard schemes to "later"; spend the saved C++-UB-debugging time on the imposition math. |
| Creep direction/magnitude wrong. | Validate against reference tools + physical proofs before locking; expose caliper + page count as explicit inputs. |
| Per-platform signing/notarization complexity (paid, externally gated). | Deferred out of M0/M1 (headless CLI has no signing payoff yet); ship unsigned artifacts + a reproducible 3-OS CI matrix first. Script codesign+notarize/Authenticode/Flatpak only once a binary has a real recipient (§18 Q7). |
| qpdf default CMake auto-links an external crypto provider; **GnuTLS is LGPL** and would taint the permissive graph. | Force the embedded **native** crypto and disable external providers (`vendored` feature; equiv. `-DREQUIRE_CRYPTO_NATIVE=1 -DUSE_IMPLICIT_CRYPTO=0 -DUSE_CRYPTO_GNUTLS=0 -DUSE_CRYPTO_OPENSSL=0`). Engine MUST NOT link GnuTLS/OpenSSL. `cargo-deny` fails CI on any copyleft in the graph. |

---

## 18. Open Questions & Decisions Pending Product Owner

Most prior open questions are resolved by the locked constraints (§1). Product identity is set:
name **Very Good Document Imposer**, distributor **Cerebral Work Institute**, license **0BSD**.
Remaining:

1. **Core PDF backend:** start on **qpdf (Apache-2.0, C++ FFI)** for proven fidelity
   (recommended), or pay an up-front cost for a **pure-Rust `lopdf` backend** to keep the
   binary 100% Rust/no-C++? *(Recommend qpdf now, trait-gated for later swap. Spike-0 decides:
   a Windows static-link failure flips this to the lopdf path.)*
2. **qpdf binding path:** extend a first-party `qpdf-sys` bindgen allowlist to `qpdfjob-c.h`, or
   re-implement form-XObject assembly directly on the C-API object model? *(Spike-0 prototypes both.)*
3. **GUI gate timing:** does the GUI begin in v1, or strictly after the engine/CLI is
   proven? *(Spec assumes v1 gate, decided then.)*
4. **JDF/CIP3 finishing output:** real near-term customer requirement (pull toward v1) or
   genuinely "later"?
5. **First-release platform lead** (mac / win / linux) — affects CI ordering and signing priority.
6. **Stock/sheet-size library scope** for M1/v1 — built-in named sheet presets vs user-defined only.
7. **Signed binaries:** when does a signed/notarized binary first have a real recipient? Until
   then, ship **unsigned** CLI artifacts; paid signing ceremony is out of M0/M1.
