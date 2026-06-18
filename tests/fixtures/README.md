# Golden fixtures

Self-authored, license-clean PDF fixtures for the M0 golden/snapshot suite (SPEC §13). These are
generated, not collected, so they can be redistributed under the repo's 0BSD license.

## Generating

```sh
cargo run -p spike0-qpdf --bin gen-fixture -- tests/fixtures/cmyk-trim.pdf 4
```

`gen-fixture` emits DeviceCMYK content with a non-trivial `TrimBox` (10..190 inside a 200×200
MediaBox) — enough to exercise TrimBox anchoring, color pass-through, and isolated-group wrapping.

## Provenance / licensing

- Everything here is produced by `gen-fixture` (our code) → safe to commit under 0BSD.
- The **Ghent Output Suite** (gwg.org) is a useful real-world reference for manual spot-checks,
  but it has **no stated redistribution license** and therefore must **not** be vendored into
  this repo. Reference it locally only.

## Planned corpus (M0 → M1)

- `cmyk-trim.pdf` — baseline DeviceCMYK + TrimBox (above).
- `rotate90-offset-trim.pdf` — `/Rotate 90` + non-zero-origin TrimBox (the placement golden).
- `spot-separation.pdf` — a Separation/DeviceN spot colorant (pass-through preservation).
- `overprint.pdf` — `OP`/`op`/`OPM` set (overprint preservation).
- `transparency-group.pdf` — page-level transparency group with an explicit `/CS`.
- `inherited-boxes.pdf` — MediaBox/Rotate inherited from the Pages node (inheritance walk).
- `no-trim.pdf` — neither TrimBox nor ArtBox (must be rejected).
