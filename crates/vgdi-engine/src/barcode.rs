//! Minimal Code 128 (Set B) encoder — pure, no PDF deps.
//!
//! Produces a symbol's *element widths* (bar/space run lengths in modules, starting with a bar) so
//! the backend can stroke it as filled rectangles. Only Code Set B (printable ASCII 32–126) is
//! implemented; QR / DataMatrix are deferred. Scanner-grade verification against a reference
//! decoder is deferred — `tests` assert the structural invariants (symbol-charge sums, start/stop,
//! checksum arithmetic) that catch encoding/transcription errors.

use vgdi_types::Rect;

/// Code 128 symbol-charge patterns, indexed by symbol value (0–106). Each string is the element
/// width sequence (bar, space, bar, …) in modules. Values 0–105 are 11-module symbols (6 elements);
/// value 106 (Stop) is a 13-module terminator (7 elements). Per ISO/IEC 15417.
#[rustfmt::skip]
const PATTERNS: [&str; 107] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212",
    "221213", "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221",
    "223211", "221132", "221231", "213212", "223112", "312131", "311222", "321122", "321221",
    "312212", "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321",
    "112313", "132113", "132311", "211313", "231113", "231311", "112133", "112331", "132131",
    "113123", "113321", "133121", "313121", "211331", "231131", "213113", "213311", "213131",
    "311123", "311321", "331121", "312113", "312311", "332111", "314111", "221411", "431111",
    "111224", "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
    "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242",
    "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311",
    "113141", "114131", "311141", "411131", "211412", "211214", "211232", "2331112",
];

const START_B: usize = 104;
const STOP: usize = 106;

/// Encode `data` as Code 128 Set B, returning the symbol's element widths in modules, alternating
/// bar, space, bar, … starting with a bar. Returns `None` if any character is outside Code-B range
/// (ASCII 32–126) or if `data` is empty.
pub fn code128b_elements(data: &str) -> Option<Vec<u8>> {
    if data.is_empty() {
        return None;
    }
    let mut values = vec![START_B];
    for ch in data.chars() {
        let c = ch as u32;
        if !(32..=126).contains(&c) {
            return None;
        }
        values.push((c - 32) as usize);
    }
    // Modulo-103 checksum: start value (weight 1) + Σ i·value_i for 1-based data positions.
    let mut sum = START_B;
    for (i, &v) in values.iter().enumerate().skip(1) {
        sum += i * v;
    }
    values.push(sum % 103);
    values.push(STOP);

    let mut elements = Vec::new();
    for &v in &values {
        for b in PATTERNS[v].bytes() {
            elements.push(b - b'0');
        }
    }
    Some(elements)
}

/// Lay a Code-128 element sequence into `area` as filled bar rectangles, scaling the module width so
/// the whole symbol fills `area` horizontally and each bar spans the full height.
pub fn bars_in_rect(elements: &[u8], area: Rect) -> Vec<Rect> {
    let total: u32 = elements.iter().map(|&e| e as u32).sum();
    if total == 0 {
        return Vec::new();
    }
    let module = area.width() / total as f64;
    let mut x = area.llx;
    let mut is_bar = true;
    let mut bars = Vec::new();
    for &e in elements {
        let w = e as f64 * module;
        if is_bar {
            bars.push(Rect::new(x, area.lly, x + w, area.ury));
        }
        x += w;
        is_bar = !is_bar;
    }
    bars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_table_has_canonical_module_charges() {
        // Every data/start/check symbol is 11 modules across 6 elements; Stop is 13 across 7.
        for (v, p) in PATTERNS.iter().enumerate() {
            let sum: u32 = p.bytes().map(|b| (b - b'0') as u32).sum();
            if v == STOP {
                assert_eq!(sum, 13, "stop pattern must be 13 modules");
                assert_eq!(p.len(), 7, "stop pattern must be 7 elements");
            } else {
                assert_eq!(sum, 11, "symbol {v} must be 11 modules");
                assert_eq!(p.len(), 6, "symbol {v} must be 6 elements");
            }
        }
    }

    #[test]
    fn rejects_non_codeb_and_empty() {
        assert!(code128b_elements("").is_none());
        assert!(code128b_elements("caf\u{e9}").is_none()); // é is outside ASCII 32–126
        assert!(code128b_elements("OK-123").is_some());
    }

    #[test]
    fn structure_starts_with_bar_and_ends_with_terminator_bar() {
        let els = code128b_elements("A").unwrap();
        // Start-B + 1 data + checksum + Stop = 3 symbols × 6 + 7 = 25 elements.
        assert_eq!(els.len(), 25);
        // First element is a bar; last is the 2-module terminator bar of Stop (even index count).
        assert_eq!(els.len() % 2, 1, "must end on a bar (odd element count)");
        let total: u32 = els.iter().map(|&e| e as u32).sum();
        // start(11) + data(11) + check(11) + stop(13) = 46 modules.
        assert_eq!(total, 46);
    }

    #[test]
    fn checksum_matches_hand_computation() {
        // "A" = value 33. checksum = (104 + 1*33) mod 103 = 137 mod 103 = 34.
        // value 34 -> PATTERN[34] = "131123". Assert that pattern appears as the 3rd symbol.
        let els = code128b_elements("A").unwrap();
        let third: Vec<u8> = els[12..18].to_vec();
        let expect: Vec<u8> = PATTERNS[34].bytes().map(|b| b - b'0').collect();
        assert_eq!(third, expect, "checksum symbol must be value 34");
    }

    #[test]
    fn bars_fill_area_width() {
        let els = code128b_elements("VGDI").unwrap();
        let area = Rect::new(10.0, 20.0, 110.0, 50.0);
        let bars = bars_in_rect(&els, area);
        assert!(!bars.is_empty());
        // All bars sit inside the area and span its full height.
        for b in &bars {
            assert!(b.llx >= 10.0 - 1e-6 && b.urx <= 110.0 + 1e-6);
            assert_eq!(b.lly, 20.0);
            assert_eq!(b.ury, 50.0);
        }
        // First bar starts at the left edge.
        assert!((bars[0].llx - 10.0).abs() < 1e-6);
    }
}
