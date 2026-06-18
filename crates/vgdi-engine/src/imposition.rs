//! Pure signature/page ordering for bound schemes (M1 design).
//!
//! Page numbers are 1-based; a value greater than the real page count is a blank pad the planner
//! drops. Surfaces are returned in print order (sheet 0 front, sheet 0 back, sheet 1 front, …).

use vgdi_types::SurfaceSide;

/// A 2-up spread = one printed surface: `[left, right]` 1-based page numbers.
pub type Spread = [usize; 2];

/// Round `n` up to the next multiple of `m` (`m == 0` → `n`).
pub fn round_up_to(n: usize, m: usize) -> usize {
    if m == 0 {
        n
    } else {
        n.div_ceil(m) * m
    }
}

/// Saddle-stitch printer-spread order for `pages` (padded up to a multiple of 4).
pub fn saddle_order(pages: usize) -> Vec<(SurfaceSide, Spread)> {
    let p = round_up_to(pages.max(1), 4);
    let sheets = p / 4;
    let mut out = Vec::with_capacity(sheets * 2);
    for i in 0..sheets {
        let front = [p - 2 * i, 1 + 2 * i];
        let back = [2 + 2 * i, p - 1 - 2 * i];
        out.push((SurfaceSide::Front, front));
        out.push((SurfaceSide::Back, back));
    }
    out
}

/// Perfect-bound order: split into gathered signatures of `sig_pages` (≥ 4, rounded to a multiple
/// of 4), each saddle-ordered, concatenated in reading order with global 1-based page numbers.
pub fn perfect_bound_order(pages: usize, sig_pages: usize) -> Vec<(SurfaceSide, Spread)> {
    let sig = round_up_to(sig_pages.max(4), 4);
    let total = round_up_to(pages.max(1), sig);
    let n_sigs = total / sig;
    let mut out = Vec::new();
    for g in 0..n_sigs {
        let base = g * sig;
        for (side, [l, r]) in saddle_order(sig) {
            out.push((side, [base + l, base + r]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn spreads(v: &[(SurfaceSide, Spread)]) -> Vec<Spread> {
        v.iter().map(|(_, s)| *s).collect()
    }

    #[test]
    fn round_up_works() {
        assert_eq!(round_up_to(1, 4), 4);
        assert_eq!(round_up_to(4, 4), 4);
        assert_eq!(round_up_to(5, 4), 8);
        assert_eq!(round_up_to(6, 8), 8);
    }

    #[test]
    fn saddle_4_pages_one_sheet() {
        let o = saddle_order(4);
        assert_eq!(o.len(), 2); // front + back
        assert_eq!(o[0], (SurfaceSide::Front, [4, 1]));
        assert_eq!(o[1], (SurfaceSide::Back, [2, 3]));
    }

    #[test]
    fn saddle_8_pages_two_sheets_classic_order() {
        let o = saddle_order(8);
        assert_eq!(
            spreads(&o),
            vec![[8, 1], [2, 7], [6, 3], [4, 5]],
            "standard saddle-stitch imposition for 8 pages"
        );
        assert_eq!(o[0].0, SurfaceSide::Front);
        assert_eq!(o[1].0, SurfaceSide::Back);
    }

    #[test]
    fn saddle_pads_to_multiple_of_four() {
        // 6 pages -> padded to 8; pages 7 and 8 are blanks (caller drops > 6).
        let o = saddle_order(6);
        assert_eq!(o.len(), 4);
        assert_eq!(spreads(&o), vec![[8, 1], [2, 7], [6, 3], [4, 5]]);
    }

    #[test]
    fn saddle_every_page_appears_exactly_once() {
        for pages in [4usize, 8, 12, 16, 20, 36, 100] {
            let o = saddle_order(pages);
            let p = round_up_to(pages, 4);
            let mut seen = BTreeSet::new();
            for (_, [l, r]) in &o {
                assert!(seen.insert(*l), "dup page {l}");
                assert!(seen.insert(*r), "dup page {r}");
            }
            assert_eq!(
                seen,
                (1..=p).collect::<BTreeSet<_>>(),
                "must be a permutation of 1..={p}"
            );
        }
    }

    #[test]
    fn perfect_bound_splits_into_signatures() {
        // 32 pages, 8 per signature -> 4 signatures, each 2 sheets (4 surfaces) = 16 surfaces.
        let o = perfect_bound_order(32, 8);
        assert_eq!(o.len(), 16);
        // signature 0 starts at page 1; signature 1 at page 9.
        assert_eq!(o[0], (SurfaceSide::Front, [8, 1]));
        assert_eq!(
            o[4],
            (SurfaceSide::Front, [16, 9]),
            "second signature offset by 8"
        );
    }

    #[test]
    fn perfect_bound_rounds_signature_size_up_to_four() {
        // sig_pages 6 -> 8; 8 pages -> exactly one signature.
        let o = perfect_bound_order(8, 6);
        assert_eq!(o.len(), 4);
        assert_eq!(o[0], (SurfaceSide::Front, [8, 1]));
    }

    #[test]
    fn perfect_bound_every_page_once_across_signatures() {
        let o = perfect_bound_order(40, 16); // 16-pad -> 48 total, 3 signatures
        let mut seen = BTreeSet::new();
        for (_, [l, r]) in &o {
            assert!(seen.insert(*l));
            assert!(seen.insert(*r));
        }
        assert_eq!(seen, (1..=48).collect::<BTreeSet<_>>());
    }
}
