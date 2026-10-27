//! Document search inside rendered Markdown preview.
//!
//! Provides text searching across rendered preview text, cycling through
//! matches, building highlights for GPUI Kit, and binary-searching block
//! spans to scroll to matches accurately and efficiently.

use std::ops::Range;

use gpui_kit::Hsla;
use gpui_kit::component::text::RangeHighlight;

/// Maximum number of match highlights to paint at once to prevent layout stalls
/// on massive documents with thousands of common occurrences.
const MAX_HIGHLIGHTS: usize = 2000;

#[derive(Debug, Default, Clone)]
pub(crate) struct DocumentSearch {
    query: String,
    matches: Vec<Range<usize>>,
    current_match_index: usize,
    cached_rendered_lower: Option<String>,
    last_rendered_len: usize,
}

impl DocumentSearch {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    #[expect(dead_code, reason = "getter for document search matches")]
    pub(crate) fn matches(&self) -> &[Range<usize>] {
        &self.matches
    }

    pub(crate) fn total_matches(&self) -> usize {
        self.matches.len()
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "used in tests"))]
    pub(crate) fn current_match_index(&self) -> usize {
        self.current_match_index
    }

    pub(crate) fn current_match(&self) -> Option<&Range<usize>> {
        self.matches.get(self.current_match_index)
    }

    pub(crate) fn match_display(&self) -> String {
        if self.matches.is_empty() {
            if self.query.is_empty() {
                String::new()
            } else {
                "No matches".to_string()
            }
        } else {
            format!("{}/{}", self.current_match_index + 1, self.matches.len())
        }
    }

    pub(crate) fn clear_query(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.current_match_index = 0;
    }

    /// Invalidate cached lowercase text (e.g. after document edits).
    pub(crate) fn invalidate_cache(&mut self) {
        self.cached_rendered_lower = None;
        self.last_rendered_len = 0;
    }

    /// Updates the query and recomputes all matching ranges against rendered preview text.
    ///
    /// Performance optimizations:
    /// 1. Caches lowercased rendered text to avoid reallocating full document strings on every search keystroke.
    /// 2. If the user is typing additional characters (the new query extends the old query),
    ///    narrows the existing match list in O(M) time rather than rescanning the whole document in O(N).
    pub(crate) fn update_query(&mut self, query: &str, rendered_text: &str) {
        let previous_query_lower = self.query.to_lowercase();
        self.query = query.to_string();

        if query.trim().is_empty() {
            self.clear_query();
            return;
        }

        let doc_changed =
            self.cached_rendered_lower.is_none() || self.last_rendered_len != rendered_text.len();
        if doc_changed {
            self.cached_rendered_lower = Some(rendered_text.to_lowercase());
            self.last_rendered_len = rendered_text.len();
        }

        let query_lower = query.to_lowercase();
        let rendered_lower = self
            .cached_rendered_lower
            .as_ref()
            .expect("cached_rendered_lower was initialized or populated above");

        // Incremental search optimization: if typing extends previous query without doc change,
        // filter the existing matches directly.
        let is_query_extension = !doc_changed
            && !previous_query_lower.is_empty()
            && query_lower.starts_with(&previous_query_lower)
            && !self.matches.is_empty();

        if is_query_extension {
            let q_len = query_lower.len();
            self.matches.retain(|range| {
                let start = range.start;
                rendered_lower.get(start..start + q_len) == Some(query_lower.as_str())
            });
            for range in &mut self.matches {
                range.end = range.start + q_len;
            }
        } else {
            self.matches = rendered_lower
                .match_indices(&query_lower)
                .map(|(ix, found)| ix..ix + found.len())
                .collect();
        }

        self.current_match_index = 0;
    }

    pub(crate) fn next_match(&mut self) -> Option<&Range<usize>> {
        if self.matches.is_empty() {
            return None;
        }
        self.current_match_index = (self.current_match_index + 1) % self.matches.len();
        self.current_match()
    }

    pub(crate) fn previous_match(&mut self) -> Option<&Range<usize>> {
        if self.matches.is_empty() {
            return None;
        }
        if self.current_match_index == 0 {
            self.current_match_index = self.matches.len() - 1;
        } else {
            self.current_match_index -= 1;
        }
        self.current_match()
    }

    /// Builds range highlights for `TextViewState`.
    ///
    /// For very large documents with thousands of matches, windows the highlights
    /// around the current match index to ensure smooth 60fps rendering while guaranteeing
    /// the active match is always prominently highlighted.
    pub(crate) fn build_highlights(&self, color: Hsla, current_color: Hsla) -> Vec<RangeHighlight> {
        let total = self.matches.len();
        if total == 0 {
            return Vec::new();
        }

        let (start_ix, end_ix) = if total <= MAX_HIGHLIGHTS {
            (0, total)
        } else {
            let half = MAX_HIGHLIGHTS / 2;
            let start = self.current_match_index.saturating_sub(half);
            let end = (start + MAX_HIGHLIGHTS).min(total);
            let start = end.saturating_sub(MAX_HIGHLIGHTS);
            (start, end)
        };

        let current_ix = self.current_match_index;
        self.matches[start_ix..end_ix]
            .iter()
            .enumerate()
            .map(|(offset, range)| {
                let actual_ix = start_ix + offset;
                RangeHighlight::new(
                    range.clone(),
                    if actual_ix == current_ix {
                        current_color
                    } else {
                        color
                    },
                )
            })
            .collect()
    }

    /// Finds the block index and rendered range for the target match range using block spans.
    #[cfg_attr(not(test), expect(dead_code, reason = "used in tests"))]
    #[expect(
        clippy::unused_self,
        reason = "helper for mapping search matches to preview blocks"
    )]
    pub(crate) fn find_block_for_target(
        &self,
        block_spans: &[Range<usize>],
        target_range: &Range<usize>,
        rendered_len: usize,
        item_count: usize,
        rendered_range_for_source: impl Fn(Range<usize>) -> Option<Range<usize>>,
    ) -> (usize, Option<Range<usize>>) {
        let (block_ix, range) = Self::find_block_index_from_ranges(
            block_spans,
            target_range.start,
            item_count,
            rendered_range_for_source,
        );

        if range.is_none() && item_count > 0 {
            let ix = if rendered_len > 0 {
                #[expect(
                    clippy::cast_precision_loss,
                    clippy::as_conversions,
                    reason = "interpolating target offset to item index"
                )]
                let ratio = (target_range.start as f32 / rendered_len as f32).clamp(0.0, 1.0);
                #[expect(
                    clippy::cast_precision_loss,
                    clippy::as_conversions,
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "interpolating target offset to item index"
                )]
                let idx = ((ratio * item_count as f32) as usize).min(item_count.saturating_sub(1));
                idx
            } else {
                0
            };
            (ix, None)
        } else {
            (block_ix, range)
        }
    }

    /// Locates the top-level block index from block source spans.
    ///
    /// Uses sequential scan for small documents (<= 16 blocks) and binary search
    /// for larger documents in O(log N) time.
    #[cfg_attr(not(test), expect(dead_code, reason = "used in tests"))]
    pub(crate) fn find_block_index_from_ranges(
        block_ranges: &[Range<usize>],
        target_offset: usize,
        item_count: usize,
        rendered_range_for_source: impl Fn(Range<usize>) -> Option<Range<usize>>,
    ) -> (usize, Option<Range<usize>>) {
        let max_items = block_ranges.len().min(item_count);
        if max_items == 0 {
            return (0, None);
        }

        if max_items <= 16 {
            let mut last_range = None;
            for (ix, source_range) in block_ranges[..max_items].iter().enumerate() {
                if let Some(rendered_range) = rendered_range_for_source(source_range.clone()) {
                    if target_offset >= rendered_range.start && target_offset <= rendered_range.end
                    {
                        return (ix, Some(rendered_range));
                    } else if target_offset < rendered_range.start {
                        return (ix.saturating_sub(1), last_range);
                    }
                    last_range = Some(rendered_range);
                }
            }
            return (max_items.saturating_sub(1), last_range);
        }

        let mut low = 0;
        let mut high = max_items;
        let mut best_ix = 0;
        let mut best_range = None;

        while low < high {
            let mid = low + (high - low) / 2;
            let mut found_range = None;

            // Probe mid; if mid does not render, probe forward
            let mut probe_ix = mid;
            while probe_ix < high {
                if let Some(r) = rendered_range_for_source(block_ranges[probe_ix].clone()) {
                    found_range = Some((probe_ix, r));
                    break;
                }
                probe_ix += 1;
            }

            if let Some((actual_ix, rendered_range)) = found_range {
                if target_offset >= rendered_range.start && target_offset <= rendered_range.end {
                    return (actual_ix, Some(rendered_range));
                } else if target_offset < rendered_range.start {
                    high = mid;
                } else {
                    best_ix = actual_ix;
                    best_range = Some(rendered_range);
                    low = actual_ix + 1;
                }
            } else {
                // No rendering blocks from mid to high
                high = mid;
            }
        }

        (best_ix, best_range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::hsla;

    #[test]
    fn test_search_navigation_index_cycling() {
        let mut search = DocumentSearch::default();
        search.update_query("target", "target one target two target three");
        assert_eq!(search.total_matches(), 3);
        assert_eq!(search.current_match_index(), 0);

        search.next_match();
        assert_eq!(search.current_match_index(), 1);
        search.next_match();
        assert_eq!(search.current_match_index(), 2);
        search.next_match();
        assert_eq!(search.current_match_index(), 0);

        search.previous_match();
        assert_eq!(search.current_match_index(), 2);
        search.previous_match();
        assert_eq!(search.current_match_index(), 1);
        search.previous_match();
        assert_eq!(search.current_match_index(), 0);
    }

    #[test]
    fn test_search_match_display_and_highlights() {
        let mut search = DocumentSearch::new();
        assert_eq!(search.match_display(), "");

        search.update_query("hello", "Hello world hello");
        assert_eq!(search.total_matches(), 2);
        assert_eq!(search.match_display(), "1/2");

        let highlights =
            search.build_highlights(hsla(0.1, 1.0, 0.5, 0.3), hsla(0.1, 1.0, 0.5, 1.0));
        assert_eq!(highlights.len(), 2);

        search.next_match();
        assert_eq!(search.match_display(), "2/2");

        search.update_query("notfound", "Hello world hello");
        assert_eq!(search.total_matches(), 0);
        assert_eq!(search.match_display(), "No matches");

        search.clear_query();
        assert_eq!(search.match_display(), "");
    }

    #[test]
    fn test_search_cached_lowercase_and_incremental_filtering() {
        let mut search = DocumentSearch::new();
        let rendered = "Alpha Beta Gamma Delta";

        search.update_query("a", rendered);
        assert_eq!(search.total_matches(), 6);
        assert!(search.cached_rendered_lower.is_some());

        search.update_query("al", rendered);
        assert_eq!(search.total_matches(), 1);

        search.update_query("alpha", rendered);
        assert_eq!(search.total_matches(), 1);

        search.invalidate_cache();
        assert!(search.cached_rendered_lower.is_none());
    }

    #[test]
    fn test_find_block_index_small_documents() {
        let block_ranges = vec![0..10, 11..30, 31..60, 61..100];
        let mapper = |r: Range<usize>| Some(r);

        let (ix, range) = DocumentSearch::find_block_index_from_ranges(&block_ranges, 5, 4, mapper);
        assert_eq!(ix, 0);
        assert_eq!(range, Some(0..10));

        let (ix, range) =
            DocumentSearch::find_block_index_from_ranges(&block_ranges, 25, 4, mapper);
        assert_eq!(ix, 1);
        assert_eq!(range, Some(11..30));

        let (ix, range) =
            DocumentSearch::find_block_index_from_ranges(&block_ranges, 80, 4, mapper);
        assert_eq!(ix, 3);
        assert_eq!(range, Some(61..100));
    }

    #[test]
    fn test_binary_search_large_documents() {
        let mut block_ranges = Vec::new();
        let mut current_offset = 0;
        for _ in 0..30 {
            block_ranges.push(current_offset..current_offset + 50);
            current_offset += 60; // 50 chars block + 10 chars gap
        }

        let mapper = |r: Range<usize>| Some(r);

        let (ix, r) = DocumentSearch::find_block_index_from_ranges(&block_ranges, 10, 30, mapper);
        assert_eq!(ix, 0);
        assert_eq!(r, Some(0..50));

        let (ix, r) = DocumentSearch::find_block_index_from_ranges(&block_ranges, 920, 30, mapper);
        assert_eq!(ix, 15);
        assert_eq!(r, Some(900..950));

        let (ix, r) = DocumentSearch::find_block_index_from_ranges(&block_ranges, 1750, 30, mapper);
        assert_eq!(ix, 29);
        assert_eq!(r, Some(1740..1790));

        let (ix, _) = DocumentSearch::find_block_index_from_ranges(&block_ranges, 295, 30, mapper);
        assert_eq!(ix, 4);
    }

    #[test]
    fn test_highlight_capping() {
        let mut search = DocumentSearch::new();
        search.matches = (0..3000).map(|i| i * 2..i * 2 + 1).collect();
        search.current_match_index = 2500;

        let highlights =
            search.build_highlights(hsla(0.0, 0.0, 0.0, 0.3), hsla(0.0, 0.0, 0.0, 1.0));
        assert_eq!(highlights.len(), MAX_HIGHLIGHTS);
        assert!(highlights.iter().any(|h| h.range() == (5000..5001)));
    }

    #[test]
    fn test_utf8_and_unicode_searching() {
        let mut search = DocumentSearch::new();
        let rendered = "Café résumé 🦀 Crab 🦀 café";

        search.update_query("café", rendered);
        assert_eq!(search.total_matches(), 2);
        assert_eq!(search.current_match(), Some(&(0..5))); // "Café" is 5 UTF-8 bytes: C(1)+a(1)+f(1)+é(2)
        assert_eq!(search.next_match(), Some(&(30..35)));

        search.update_query("🦀", rendered);
        assert_eq!(search.total_matches(), 2);

        // Incremental search extension with multi-byte unicode characters adjacent to matches
        let text_with_multibyte = "a・b and ab";
        search.update_query("a", text_with_multibyte);
        assert_eq!(search.total_matches(), 3);
        // Typing "ab" narrows the matches. Index into "a・" must not panic on char boundary.
        search.update_query("ab", text_with_multibyte);
        assert_eq!(search.total_matches(), 1);
        assert_eq!(search.current_match(), Some(&(10..12)));
    }

    #[test]
    fn test_empty_and_whitespace_queries() {
        let mut search = DocumentSearch::new();
        let rendered = "Some document text here";

        search.update_query("", rendered);
        assert_eq!(search.total_matches(), 0);
        assert_eq!(search.match_display(), "");

        search.update_query("   ", rendered);
        assert_eq!(search.total_matches(), 0);
        assert_eq!(search.match_display(), "");

        search.update_query("document", rendered);
        assert_eq!(search.total_matches(), 1);
        assert_eq!(search.match_display(), "1/1");

        search.clear_query();
        assert_eq!(search.total_matches(), 0);
        assert_eq!(search.match_display(), "");
    }

    #[test]
    fn test_find_block_for_target_fallback() {
        let search = DocumentSearch::new();
        let block_spans: Vec<Range<usize>> = Vec::new();
        let target_range = 50..55;
        let mapper = |r: Range<usize>| Some(r);

        // When block spans are empty, falls back to proportional estimate
        let (ix, range) =
            search.find_block_for_target(&block_spans, &target_range, 100, 10, mapper);
        assert_eq!(ix, 5); // 50 / 100 * 10 = 5
        assert!(range.is_none());
    }
}
