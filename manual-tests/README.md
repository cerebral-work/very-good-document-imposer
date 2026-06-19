# manual-tests

Hand-run prepress sanity checks for the imposition engine: JobSpecs you impose and eyeball, not part
of `cargo test`. Source fixtures (`src-*.pdf`) and outputs (`out/`) are gitignored — regenerate the
sources with the throwaway `gen-fixture` tool (`spikes/spike0-qpdf`):

    cargo build -p spike0-qpdf --bin gen-fixture
    BIN=target/debug/gen-fixture
    $BIN manual-tests/src-1p.pdf 1
    $BIN manual-tests/src-4p.pdf 4
    $BIN manual-tests/src-6p.pdf 6
    $BIN manual-tests/src-16p.pdf 16
    $BIN manual-tests/src-4p-bleed.pdf 4 bleed     # adds a BleedBox + contrasting bleed band
    $BIN manual-tests/src-6p-bleed.pdf 6 bleed

Run a job (rebuilds the CLI as needed):

    cargo run -p vgdi-cli -- manual-tests/01-nup-full-marks.json -o manual-tests/out/01.pdf

Each `*.json` exercises a facet: `01` full mark set on n-up, `02` step & repeat, `03a/03b` bleed-pull
on/off, `04` saddle (fold + spine-safe bleed), `05` perfect-bound (collation), `06` saddle bleed,
`metamorphosis-booklet` a perfect-bound run cross-checked against Quite Imposing (needs an external
source PDF — edit the `path`).
