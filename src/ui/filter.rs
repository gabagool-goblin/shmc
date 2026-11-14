//! The list filter for Queue, Browser and Playlists: case-insensitive,
//! NFKC-normalized substring matching over the loaded rows.

use ratatui::style::Style;
use ratatui::text::Span;
use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Default)]
pub struct ListFilter {
    /// The input has keyboard focus.
    pub typing: bool,
    pub text: String,
    /// Real indices of matching rows; meaningful only while `text` is non-empty.
    pub indices: Vec<usize>,
}

impl ListFilter {
    /// A non-empty needle is active even when it matches nothing.
    pub fn is_active(&self) -> bool {
        !self.text.is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.indices.clear();
        self.typing = false;
    }

    /// Recompute matches; call on every keystroke and whenever the list changes.
    pub fn apply(&mut self, len: usize, mut row_matches: impl FnMut(usize) -> bool) {
        if self.text.is_empty() {
            self.indices.clear();
            return;
        }
        self.indices = (0..len).filter(|&i| row_matches(i)).collect();
    }

    pub fn len(&self, total: usize) -> usize {
        if self.text.is_empty() {
            total
        } else {
            self.indices.len()
        }
    }

    /// Filtered cursor -> real index; not for an empty filtered view.
    pub fn real(&self, cursor: usize, total: usize) -> usize {
        if self.text.is_empty() {
            cursor.min(total.saturating_sub(1))
        } else {
            *self.indices.get(cursor).unwrap_or(&0)
        }
    }

    /// Real index -> filtered cursor, to keep the cursor on its item across a refresh.
    pub fn cursor_of(&self, real: usize) -> Option<usize> {
        if self.text.is_empty() {
            Some(real)
        } else {
            self.indices.iter().position(|&i| i == real)
        }
    }

    /// The "filter: text (n/m)" line, None when inactive.
    pub fn indicator(&self, total: usize) -> Option<String> {
        if self.text.is_empty() {
            return None;
        }
        Some(format!("filter: {} ({}/{})", self.text, self.indices.len(), total))
    }
}

/// NFKC-normalized, lowercased `s`, with each char's original range. Combining
/// marks are normalized with their base char, as they are in the needle.
fn normalize_mapped(s: &str) -> (Vec<char>, Vec<(usize, usize)>) {
    let chars: Vec<char> = s.chars().collect();
    let mut norm = Vec::new();
    let mut orig = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let mut j = i + 1;
        while j < chars.len() {
            let produced: Vec<char> = chars[j].to_string().nfkc().collect();
            if !produced.is_empty() && produced.iter().all(|&c| canonical_combining_class(c) != 0) {
                j += 1;
            } else {
                break;
            }
        }
        let chunk: String = chars[i..j].iter().collect();
        for nc in chunk.nfkc() {
            for lc in nc.to_lowercase() {
                norm.push(lc);
                orig.push((i, j));
            }
        }
        i = j;
    }
    (norm, orig)
}

fn normalize(s: &str) -> Vec<char> {
    s.nfkc().flat_map(|c| c.to_lowercase()).collect()
}

/// A needle normalized once, to test many rows against.
pub struct Needle(Vec<char>);

impl Needle {
    pub fn new(needle: &str) -> Self {
        Needle(normalize(needle))
    }

    pub fn found_in(&self, text: &str) -> bool {
        self.0.is_empty() || find(&normalize_mapped(text).0, &self.0).is_some()
    }
}

pub fn matches(text: &str, needle: &str) -> bool {
    Needle::new(needle).found_in(text)
}

/// Char-index ranges (in the ORIGINAL text) of every match, non-overlapping.
pub fn match_ranges(text: &str, needle: &str) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let (hay, orig) = normalize_mapped(text);
    let ndl = normalize(needle);
    let mut ranges = Vec::new();
    let mut from = 0usize;
    while from + ndl.len() <= hay.len() {
        if let Some(pos) = find(&hay[from..], &ndl) {
            let start = from + pos;
            let end = start + ndl.len(); // exclusive in normalized space
            let o_start = orig[start].0;
            let o_end = orig[end - 1].1; // exclusive in original chars
                                         // One original char can normalize to several matches ("Ⅲ" is
                                         // "iii"): merge them, or `highlight` draws that char again.
            match ranges.last_mut() {
                Some((_, last_end)) if o_start < *last_end => *last_end = o_end.max(*last_end),
                _ => ranges.push((o_start, o_end)),
            }
            from = end.max(start + 1);
        } else {
            break;
        }
    }
    ranges
}

fn find(hay: &[char], ndl: &[char]) -> Option<usize> {
    if ndl.is_empty() || ndl.len() > hay.len() {
        return None;
    }
    hay.windows(ndl.len()).position(|w| w == ndl)
}

/// Split `text` into spans, styling the matches with `hit`.
pub fn highlight<'a>(text: &'a str, needle: &str, base: Style, hit: Style) -> Vec<Span<'a>> {
    let ranges = match_ranges(text, needle);
    if ranges.is_empty() {
        return vec![Span::styled(text.to_string(), base)];
    }
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut pos = 0usize;
    for (s, e) in ranges {
        if s > pos {
            spans.push(Span::styled(chars[pos..s].iter().collect::<String>(), base));
        }
        spans.push(Span::styled(chars[s..e].iter().collect::<String>(), hit));
        pos = e;
    }
    if pos < chars.len() {
        spans.push(Span::styled(chars[pos..].iter().collect::<String>(), base));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_insensitive_substring() {
        assert!(matches("Gorgeous Power", "gorg"));
        assert!(matches("Gorgeous Power", "POWER"));
        assert!(!matches("Gorgeous Power", "xyz"));
        assert!(matches("anything", ""), "empty needle matches all");
    }

    #[test]
    fn cjk_substring() {
        assert!(matches("デスロウ", "デス"));
        assert!(matches("物哀いスペースシャトル", "スペ"));
        assert!(!matches("デスロウ", "ゴ"));
    }

    #[test]
    fn nfkc_fullwidth_and_halfwidth() {
        // full-width latin vs ascii needle
        assert!(matches("ＧＯＲＧＯＮ", "gorgon"));
        // half-width katakana folds to full-width (ﾃ is TE, not DE)
        assert!(matches("ﾃｽﾛｳ", "テスロウ"));
        assert!(matches("テスロウ", "ﾃｽﾛｳ"));
        // a half-width dakuten pair composes to the voiced char under NFKC
        assert!(matches("ﾃﾞｽ", "デス"));
        // voicing still matters: テ does not match デ
        assert!(!matches("ﾃｽ", "デス"));
    }

    #[test]
    fn decomposed_accents_match_composed() {
        assert!(matches("Cafe\u{301}", "é"));
        assert!(matches("Café", "e\u{301}"));
        assert_eq!(match_ranges("Cafe\u{301} x", "é"), vec![(3, 5)]);
    }

    #[test]
    fn match_ranges_maps_back_to_original_chars() {
        // highlight spans must cover the original (wide) characters
        let r = match_ranges("Starfall!", "fall");
        assert_eq!(r, vec![(4, 8)]);
        let r = match_ranges("デスロウ", "デス");
        assert_eq!(r, vec![(0, 2)], "wide chars map by char index");
        // two matches
        let r = match_ranges("abc abc", "ab");
        assert_eq!(r, vec![(0, 2), (4, 6)]);
    }

    #[test]
    fn multi_match_char_drawn_once() {
        // NFKC turns "Ⅲ" into "iii": three matches, one original char.
        assert_eq!(match_ranges("Ⅲ x", "i"), vec![(0, 1)]);
        let spans = highlight("FF Ⅲ", "i", Style::default(), Style::default());
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "FF Ⅲ");
    }

    #[test]
    fn highlight_spans_cover_matches_and_keep_vs16() {
        let base = Style::default();
        let hit = Style::default();
        let spans = highlight("Cyan Crimson", "c", base, hit);
        // "C" (hit) + "yan " (base) + "C" (hit) + "rimson" (base)
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].content, "C");
        assert_eq!(spans[2].content, "C");
        // multi-word needle
        let spans = highlight("aaa bbb ccc", "bbb", base, hit);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "aaa bbb ccc");
        // VS16 stays attached to its base when highlighted
        let spans = highlight("\u{263a}\u{fe0f}x", "\u{263a}", base, hit);
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "\u{263a}\u{fe0f}x");
    }

    #[test]
    fn filter_index_mapping() {
        let mut f = ListFilter::default();
        let items = ["one", "two", "three", "tone"];
        f.text = "t".into();
        let text = f.text.clone();
        f.apply(items.len(), |i| matches(items[i], &text));
        assert_eq!(f.indices, vec![1, 2, 3]);
        assert_eq!(f.len(4), 3);
        assert_eq!(f.real(0, 4), 1);
        assert_eq!(f.real(2, 4), 3);
        assert_eq!(f.cursor_of(3), Some(2));
        assert_eq!(f.cursor_of(0), None);

        // inactive filter is identity
        f.clear();
        assert_eq!(f.real(2, 4), 2);
        assert_eq!(f.cursor_of(3), Some(3));
        assert_eq!(f.len(4), 4);
    }

    #[test]
    fn inactive_filter_is_identity() {
        let f = ListFilter::default();
        assert!(!f.is_active());
        assert_eq!(f.real(2, 4), 2);
        assert_eq!(f.cursor_of(3), Some(3));
        assert_eq!(f.len(4), 4);
        assert_eq!(f.indicator(4), None);
    }

    #[test]
    fn active_without_matches_is_empty_not_inactive() {
        let mut f = ListFilter { text: "zzz".into(), ..ListFilter::default() };
        let text = f.text.clone();
        f.apply(4, |i| matches(["one", "two", "three", "tone"][i], &text));
        assert!(f.is_active(), "non-empty needle stays active");
        assert_eq!(f.len(4), 0, "no matches -> empty list");
        assert_eq!(f.cursor_of(0), None);
        // indicator distinguishes "(0/4)" from the inactive None
        assert_eq!(f.indicator(4).as_deref(), Some("filter: zzz (0/4)"));
    }
}
