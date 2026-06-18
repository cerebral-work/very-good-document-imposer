# ADR-0001 — Platform & Technology Stack

> Status: **Proposed** · 2026-06-18 (revised post-adversarial-review: qpdf binding reality, crypto-provider constraint, Spike-0 promoted to first gate).
> Decision record for the engine language, PDF core, UI direction, and licensing posture
> of very-good-document-imposer. Backs §3, §5, §6 of `SPEC.md`.

## Context

Greenfield cross-platform imposition tool. Locked constraints (product owner):
macOS + Windows + Linux; native-feeling UI; **no Electron / no webview**; **not** an Acrobat
plug-in; **maximally permissive ("0BSD-maxxing")** licensing; **engine/CLI first, GUI later**;
**pro-prepress** fidelity bar; **PDF-only** input for v1.

A multi-agent research pass (5 research lenses + 5 stack evaluations + synthesis) informed
this record.

## Decision

**Decouple the engine-language decision from the UI-toolkit decision** — they have different
winners, and the only real risk lives in how they join.

1. **Engine = Rust** (pure, permissive, no UI deps). Memory safety turns determinism-critical
   geometry bugs (CTM, `/Rotate`, TrimBox anchoring, creep, blend-space) into `Result`s
   rather than silent wrong-placement UB. The product *is* the geometry math, so this safety
   property is worth more here than almost anywhere.
2. **PDF core = qpdf ≥ 7 (Apache-2.0)** for verbatim page→Form-XObject placement, behind an
   `ImpositionWriter` trait. qpdf's foreign-object copy is the one mechanism purpose-built for
   *lossless* content+resource copy (CMYK/spot/ICC/overprint survive; an existing page `/Group`
   is copied), and Apache-2.0 statically links cleanly into a 0BSD binary.
   **Binding reality (do not gloss):** the page→form-XObject helper
   (`QPDFPageObjectHelper::getFormXObjectForPage`) is **C++-only** — it is *not* in `qpdf-c.h`
   nor the pre-1.0, single-maintainer `qpdf` Rust crate (0.3.x). We therefore own the binding
   **first-party from day one**: either (a) extend a forked `qpdf-sys` bindgen allowlist to
   `qpdfjob-c.h`, or (b) re-implement form-XObject assembly on the C-API object model
   (`qpdf_oh_new_stream` + `qpdf_oh_copy_foreign_object` + page content data + manual
   `/Group`,`/BBox`,`/Resources`,`/Matrix`). The **isolated transparency-group wrapper** (blend
   `/CS`, `/I true`) is *engine work* — qpdf does not synthesize it. Build with the crate's
   `vendored` feature and **force native crypto** (disable GnuTLS/OpenSSL auto-link — GnuTLS is
   LGPL). A pure-Rust `lopdf` backend is a **from-scratch reimplementation** (not a drop-in),
   gated to pass the same golden suite; it becomes the path only if Spike-0's Windows static-link
   fails.
3. **Marks authoring** via direct content-stream emit (or `pdf-writer`). Note: `pdf-writer`/
   `krilla` **cannot embed external PDF pages** → marks only, never source placement.
4. **CLI is the v1 deliverable and the automation contract.** Engine exposed as Rust lib +
   stable C ABI + CLI from one codebase.
5. **GUI deferred.** When the phase opens, the toolkit must be native-feeling **and**
   permissive. **Slint is GPL/commercial → excluded.** Candidate set then: Qt Widgets
   (LGPL, dynamic-link — best native canvas via QGraphicsView) or a permissive alternative
   (wxWidgets / Avalonia-MIT / egui). Decided at the gate, not now.
6. **Coarse, data-only seam** between engine and any UI: submit JobSpec JSON → receive
   ImpositionPlan + render preview bitmap. **No live PDF objects or QObjects cross FFI.**

## Stack evaluation scorecards

Scores 0–10 per axis; weighted total 0–100 (native UI, PDF fidelity, licensing weighted high).
These evaluated *full* GUI stacks; under our **engine/CLI-first + 0BSD** constraints the UI
axis is deferred, which is why the chosen direction differs from the single highest raw score.

| Stack | Native UI | License | Fidelity | Preview | Velocity | X-plat | Maturity | Hiring | **Total** |
|---|:--:|:--:|:--:|:--:|:--:|:--:|:--:|:--:|:--:|
| C++ + Qt (Widgets/QML) | 9 | 7 | 8 | 7 | 5 | 9 | 9 | 6 | **80** |
| Rust engine + Qt (CXX-Qt) | 9 | 5 | 9 | 8 | 4 | 6 | 5 | 4 | **68** |
| Rust engine + Slint | 4 | 8 | 9 | 6 | 6 | 7 | 6 | 4 | **66** |
| C++ engine + GTK4 / per-OS native | 5 | 5 | 8 | 8 | 3 | 5 | 8 | 4 | **56** |
| C#/.NET + Avalonia | 5 | 6 | 4 | 6 | 9 | 7 | 6 | 8 | **55** |

### Why the raw winner (C++/Qt, 80) is the **runner-up**, not the choice
- Highest native-UI + maturity, and PoDoFo's `PdfXObjectForm` + podofoimpose are a real
  imposition precedent — but podofoimpose is **GPL-2.0** (reference only, never copy).
- **C++ memory-unsafety in a determinism-critical engine** surfaces as *silent mis-placement*
  on specific customer PDFs — the exact failure mode this product cannot tolerate.
- Under **engine/CLI-first**, the Qt-monolith advantage (one language, no FFI) is largely a
  *UI-phase* benefit we're deferring anyway. Picking it now would couple the engine to a UI
  toolkit decision we explicitly want to postpone.
- Pick this **only** if the team is already deeply C++/Qt-fluent and wants a single-language
  monolith more than memory-safe geometry.

### Why not Rust+Qt direct bind (CXX-Qt, 68)
Highest *ceiling*, but the **Rust↔Qt bridge is the one component no research vouches for**.
CXX-Qt steers toward QML (less native) over Widgets (more native, FFI-hostile from Rust), and
realistically degenerates into a two-language project anyway. We get the same end state more
cleanly via the **coarse data-only seam** (engine cdylib + CLI; UI is just a client) without a
fragile live-object bridge — so this is subsumed by our chosen architecture rather than adopted
as-is.

### Why not Slint (66)
Strong engine, wrong UI: Slint is **self-drawn, not native widgets**, immature mac/Windows
styling, no document scene-graph (you rebuild pan/zoom/hit-test from scratch), and **GPL/commercial
licensing** that fails our permissive policy. We keep the Rust engine, drop Slint.

### Why not GTK4 / per-OS native (56)
"GTK4 as the one toolkit" is native on **Linux only**; true cross-OS native means a 3× per-OS UI
cost. Its standout (MuPDF CMYK preview) is **AGPL/commercial → excluded**. Best only for an
*AGPL open-source* build, which we are not.

### Why not Avalonia/.NET (55)
Fastest velocity + deepest hiring pool, MIT UI — but **.NET has no permissive prepress-grade PDF
placement+color engine**. You end up P/Invoking native PDFium+lcms2 anyway (losing the managed
advantage), with PDFium's less-battle-tested overprint/spot/transparency rewrite as the core
correctness risk. The load-bearing transform belongs in a stack with better permissive PDF libs
(Rust qpdf/lopdf).

## Consequences

**Positive**
- Fully permissive, statically-linkable engine/CLI — clean 0BSD posture; no fees, no copyleft.
- Memory-safe geometry core with byte-stable, golden-testable output.
- UI toolkit decision stays orthogonal and deferred; the CLI is a complete product on its own.
- One engine powers CLI, future GUI, hot-folder, and scripting alike.

**Negative / costs**
- A C++ FFI dependency (qpdf) inside an otherwise pure-Rust engine — mitigated by the trait
  boundary + CI static-linking + a pure-Rust fallback backend.
- **No accurate CMYK/overprint soft-proof** is possible within the permissive policy; the future
  GUI gets honest RGB preview only.
- Thin hiring intersection (Rust + prepress PDF expertise); mitigated because the coarse seam lets
  engine and UI be staffed separately against a JSON contract.

## Follow-ups
- **Spike-0 is the first sequenced gate** (SPEC §16): statically link qpdf + place one page as a
  transformed Form XObject, green in CI on macOS, Windows (MSVC), and Linux. Windows MSVC +
  C++/zlib/jpeg static-link is the classic failure point; a failure here is the trigger to
  evaluate the lopdf reimplementation **before** any scheme code.
- Vendor + pin a qpdf SHA (≥ 7; target latest 12.x). Record that pre-7 is dual-available as
  Artistic-2.0 so license scanners don't false-positive. Plan to vendor/fork the binding rather
  than depend on the upstream `qpdf` crate's cadence.
- Enforce the crypto-provider constraint in the build (native only; no GnuTLS/OpenSSL) and run
  `cargo-deny` in CI to fail on any copyleft entering the graph.
- Stand up a **self-authored, license-clean** golden corpus (DeviceN/Separation/overprint/spot/
  transparency) before scheme work; reference the Ghent Output Suite for manual spot-checks only
  (not vendored — no stated redistribution license).
- Re-open the GUI toolkit choice as **ADR-0002** at the GUI gate.
