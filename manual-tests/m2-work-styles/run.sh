#!/usr/bin/env bash
# Build the engine, generate front/back source PDFs, and impose the M2 work-style jobs so you can
# eyeball the duplex back surface. Run from anywhere: it cd's to the repo root itself.
#
#   bash manual-tests/m2-work-styles/run.sh
#
# Outputs (PDFs + per-surface PNGs) land in manual-tests/out/m2/ (gitignored).
set -euo pipefail

cd "$(dirname "$0")/../.."
HERE=manual-tests/m2-work-styles
OUT=manual-tests/out/m2
mkdir -p "$OUT"

# The vendored qpdf build needs the Xcode toolchain on this Mac (see STATUS.md "Build / test / run").
if [ -d /Applications/Xcode.app ]; then
  export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
  export LIBCLANG_PATH=/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib
fi

echo "==> building vgdi-cli + gen-fixture"
cargo build -q -p vgdi-cli -p spike0-qpdf

# Two equal-geometry sources (TrimBox 10..190, BleedBox 5..195), tint varying per page so the
# position reflection is visible. v1 requires equal trim+bleed, which both fixtures satisfy.
echo "==> generating front/back source PDFs (4 pages each, with bleed)"
cargo run -q -p spike0-qpdf --bin gen-fixture -- "$OUT/front.pdf" 4 bleed
cargo run -q -p spike0-qpdf --bin gen-fixture -- "$OUT/back.pdf"  4 bleed

render() { gs -q -dBATCH -dNOPAUSE -sDEVICE=png16m -r96 -o "$OUT/$1-%d.png" "$OUT/$1.pdf"; }

for job in nup-sheetwise nup-work-and-turn nup-tumble nup-perfector gang-work-and-turn; do
  echo "==> imposing $job  (page 1 = front surface, page 2 = back surface)"
  cargo run -q -p vgdi-cli -- "$HERE/$job.json" -o "$OUT/$job.pdf"
  command -v gs >/dev/null && render "$job"
done

# Cell-derived marks (crop) reflect correctly on every style. Sheet-edge furniture (slug/colour-bar/
# barcode) on a gripper-moving style (tumble/perfector) is rejected until the gripper-edge model lands.
echo "==> imposing nup-tumble-slug  (EXPECTED to fail — furniture on a moved gripper)"
if cargo run -q -p vgdi-cli -- "$HERE/nup-tumble-slug.json" -o "$OUT/nup-tumble-slug.pdf" 2>"$OUT/furniture.err"; then
  echo "   !! unexpected success"
else
  echo "   rejected as designed: $(cat "$OUT/furniture.err")"
fi

echo
echo "Done. Open the work-and-turn result and compare its two pages:"
echo "   open $OUT/nup-work-and-turn.pdf"
echo "Or compare the rendered surfaces:"
echo "   open $OUT/nup-work-and-turn-1.png $OUT/nup-work-and-turn-2.png"
