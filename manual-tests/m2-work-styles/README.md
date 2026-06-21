# M2 work styles — manual test

Eyeball the duplex back surface that `WorkStyle` drives on N-up / Step & Repeat. All four styles are
wired: `sheetwise`, `work-and-turn`, `work-and-tumble`, `perfector`. Cell-derived marks (crop, etc.)
reflect with the cells on every style; **sheet-edge furniture** (slug / colour bar / barcode) on a
gripper-moving style (tumble / perfector) is still rejected until the gripper-edge model lands.

## Run

```sh
bash manual-tests/m2-work-styles/run.sh
```

It builds the CLI, generates two equal-geometry source PDFs (`front.pdf`, `back.pdf`, 4 pages each
with a per-page tint so position is visible), imposes each job, and renders every surface to PNG.
All outputs go to `manual-tests/out/m2/` (gitignored). Each imposed PDF has **2 pages per sheet**:
page 1 = front surface, page 2 = back surface.

## What to look for

Every job's page 1 is the front (2×2 grid, row-major, light → dark); page 2 is the back surface.

- **`nup-work-and-turn`** — back is the front **mirrored left-to-right** (columns swap about the
  vertical centreline), each cell upright (positions reflect, content never mirrors, SPEC §9).
- **`nup-sheetwise`** — back sits at the **same positions** as the front (its own independent grid, no
  reflection). Diff against `nup-work-and-turn` to see what the work style changes.
- **`nup-tumble`** — back is the front **mirrored top-to-bottom** (about the horizontal centreline),
  cells upright.
- **`nup-perfector`** — back is the front **rotated 180°** about the sheet centre: positions reflect on
  *both* axes and each cell's content is turned 180° (upside-down), unlike turn/tumble.
- **`gang-work-and-turn`** — a Step & Repeat card gang (2×3, bleed-to-bleed) with a back. Two surfaces,
  back gang reflected. With these identical fixtures the cells are uniform so the mirror isn't visually
  obvious — swap in your own distinct front/back card art (below) to see it.
- **`nup-tumble-slug`** — expected to **fail**: tumble + a slug → `sheet-edge furniture … needs the
  gripper-edge model …`. Cell marks (crop) are fine on tumble/perfector; furniture isn't yet.

## Use your own art

Edit the `path` fields in any job JSON to point at real PDFs (e.g. business-card front/back). v1
constraint: each back page's **TrimBox and BleedBox must match its paired front page's size** (else
the job is rejected with a geometry-mismatch error), and the back source must have the **same page
count** as the front.
