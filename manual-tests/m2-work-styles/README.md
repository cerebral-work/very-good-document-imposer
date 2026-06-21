# M2 work styles — manual test

Eyeball the duplex back surface added in M2 Phase 1 (`WorkStyle` driving N-up / Step & Repeat).
Phase 1 ships the two **gripper-preserving** styles: `sheetwise` and `work-and-turn`. Tumble and
perfector are rejected until Phase 2.

## Run

```sh
bash manual-tests/m2-work-styles/run.sh
```

It builds the CLI, generates two equal-geometry source PDFs (`front.pdf`, `back.pdf`, 4 pages each
with a per-page tint so position is visible), imposes each job, and renders every surface to PNG.
All outputs go to `manual-tests/out/m2/` (gitignored). Each imposed PDF has **2 pages per sheet**:
page 1 = front surface, page 2 = back surface.

## What to look for

- **`nup-work-and-turn`** — the headline. Page 1 (front) fills the 2×2 grid row-major, light → dark.
  Page 2 (back) is page 1 **mirrored left-to-right**: the columns swap, but each cell's content stays
  **upright** (the squares aren't flipped — positions reflect, content never mirrors, SPEC §9).
- **`nup-sheetwise`** — same job, `work_style: sheetwise`. Page 2 (back) sits at the **same positions**
  as page 1 (its own independent grid, no reflection). Diff it against `nup-work-and-turn` to see what
  the work style changes.
- **`gang-work-and-turn`** — a Step & Repeat card gang (2×3, bleed-to-bleed) with a back. Two surfaces,
  back gang reflected. With these identical fixtures the cells are uniform so the mirror isn't visually
  obvious — swap in your own distinct front/back card art (below) to see it.
- **`nup-tumble`** — expected to **fail** with `work style 'work-and-tumble' not yet supported …`,
  demonstrating the Phase-1 guard (no silently-wrong output).

## Use your own art

Edit the `path` fields in any job JSON to point at real PDFs (e.g. business-card front/back). v1
constraint: each back page's **TrimBox and BleedBox must match its paired front page's size** (else
the job is rejected with a geometry-mismatch error), and the back source must have the **same page
count** as the front.
