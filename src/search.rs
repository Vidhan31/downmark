//! Document search inside rendered Markdown preview.
//!
//! Provides text and regular-expression searching across rendered preview text,
//! cycling through matches, building highlights for GPUI Kit, and binary-searching
//! block spans to scroll to matches accurately and efficiently.

use std::{borrow::Cow, ops::Range};

use gpui_kit::{Hsla, component::text::RangeHighlight};
use regex::{Regex, RegexBuilder};

/// Maximum number of match highlights to paint at once to prevent layout stalls
/// on massive documents with thousands of common occurrences.
const MAX_HIGHLIGHTS: usize = 2000;

/// Maximum number of matches retained per query to bound memory and UI work
/// on broad patterns (e.g., `.` in regex mode on large documents).
const MAX_MATCHES: usize = 20_000;

/// Toggleable matching modes for [`DocumentSearch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchOption {
    CaseSensitive,
    WholeWord,
    Regex,
}

/// Matching options controlling how [`DocumentSearch`] interprets a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct SearchOptions {
    pub(crate) case_sensitive: bool,
    pub(crate) whole_word: bool,
    pub(crate) regex: bool,
}

impl SearchOptions {
    /// Flips the given search option flag.
    pub(crate) fn toggle(&mut self, option: SearchOption) {
        match option {
            SearchOption::CaseSensitive => self.case_sensitive = !self.case_sensitive,
            SearchOption::WholeWord => self.whole_word = !self.whole_word,
            SearchOption::Regex => self.regex = !self.regex,
        }
    }
}

#[derive(Debug, Clone)]
enum CompiledMatcher {
    /// Exact literal substring search via `str::match_indices` (zero compile, SIMD-accelerated).
    LiteralExact(String),
    /// Case-insensitive literal or regular-expression search via linear-time `regex::Regex`.
    Regex(Regex),
}

#[derive(Debug, Default, Clone)]
pub(crate) struct DocumentSearch {
    query: String,
    options: SearchOptions,
    matches: Vec<Range<usize>>,
    matches_truncated: bool,
    current_match_index: usize,
    error: Option<String>,
    compiled_for: Option<(String, SearchOptions)>,
    compiled: Option<CompiledMatcher>,
}

impl DocumentSearch {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn options(&self) -> SearchOptions {
        self.options
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn matches(&self) -> &[Range<usize>] {
        &self.matches
    }

    pub(crate) fn total_matches(&self) -> usize {
        self.matches.len()
    }

    pub(crate) fn current_match_index(&self) -> usize {
        self.current_match_index
    }

    pub(crate) fn current_match(&self) -> Option<&Range<usize>> {
        self.matches.get(self.current_match_index)
    }

    pub(crate) fn match_display(&self) -> String {
        if self.error.is_some() {
            return "Invalid regex".to_string();
        }
        if self.matches.is_empty() {
            if self.query.trim().is_empty() {
                String::new()
            } else {
                "No matches".to_string()
            }
        } else if self.matches_truncated {
            format!("{}/{}+", self.current_match_index + 1, self.matches.len())
        } else {
            format!("{}/{}", self.current_match_index + 1, self.matches.len())
        }
    }

    /// Clears the active query and match state while preserving search options.
    pub(crate) fn clear_query(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.matches_truncated = false;
        self.current_match_index = 0;
        self.error = None;
        self.compiled_for = None;
        self.compiled = None;
    }

    /// Updates the query and recomputes all matching ranges against rendered preview text.
    ///
    /// Preserves the nearest match position when the query and options are unchanged
    /// (for example, after a document edit refreshes the preview).
    pub(crate) fn update_query(&mut self, query: &str, rendered_text: &str) {
        let same_spec = self
            .compiled_for
            .as_ref()
            .is_some_and(|(prev_q, prev_opts)| prev_q == query && *prev_opts == self.options);
        let prev_offset = same_spec
            .then(|| self.current_match().map(|range| range.start))
            .flatten();

        self.query = query.to_string();
        self.recompute(rendered_text, prev_offset);
    }

    /// Replaces the active search options and recomputes matches against `rendered_text`.
    #[cfg_attr(not(test), expect(dead_code, reason = "used in tests"))]
    pub(crate) fn set_options(&mut self, options: SearchOptions, rendered_text: &str) {
        if self.options == options {
            return;
        }
        let prev_offset = self.current_match().map(|range| range.start);
        self.options = options;
        self.recompute(rendered_text, prev_offset);
    }

    /// Toggles one search option and recomputes matches against `rendered_text`.
    pub(crate) fn toggle_option(&mut self, option: SearchOption, rendered_text: &str) {
        let prev_offset = self.current_match().map(|range| range.start);
        self.options.toggle(option);
        self.recompute(rendered_text, prev_offset);
    }

    /// Finds matches for the current compiled query and options in an arbitrary text buffer
    /// (such as the Markdown source for the editor overview ruler), capped at `limit`.
    pub(crate) fn find_matches_in(&self, text: &str, limit: usize) -> Vec<Range<usize>> {
        let Some(matcher) = &self.compiled else {
            return Vec::new();
        };
        collect_matches(matcher, self.options, text, limit).0
    }

    fn recompute(&mut self, rendered_text: &str, preserve_near_offset: Option<usize>) {
        if self.query.trim().is_empty() {
            let options = self.options;
            self.clear_query();
            self.options = options;
            return;
        }

        let needs_compile = self
            .compiled_for
            .as_ref()
            .is_none_or(|(q, opts)| q != &self.query || *opts != self.options);

        if needs_compile {
            match compile_matcher(&self.query, self.options) {
                Ok(matcher) => {
                    self.error = None;
                    self.compiled = Some(matcher);
                    self.compiled_for = Some((self.query.clone(), self.options));
                }
                Err(err) => {
                    self.error = Some(err);
                    self.compiled = None;
                    self.compiled_for = Some((self.query.clone(), self.options));
                    self.matches.clear();
                    self.matches_truncated = false;
                    self.current_match_index = 0;
                    return;
                }
            }
        }

        if let Some(matcher) = &self.compiled {
            let (matches, truncated) =
                collect_matches(matcher, self.options, rendered_text, MAX_MATCHES);
            self.matches = matches;
            self.matches_truncated = truncated;
        } else {
            self.matches.clear();
            self.matches_truncated = false;
        }

        self.current_match_index = match preserve_near_offset {
            Some(offset) if !self.matches.is_empty() => self
                .matches
                .partition_point(|range| range.start < offset)
                .min(self.matches.len() - 1),
            _ => 0,
        };
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
                high = mid;
            }
        }

        (best_ix, best_range)
    }
}

fn compile_matcher(query: &str, options: SearchOptions) -> Result<CompiledMatcher, String> {
    if !options.regex && options.case_sensitive {
        return Ok(CompiledMatcher::LiteralExact(query.to_string()));
    }

    let pattern = if options.regex {
        Cow::Borrowed(query)
    } else {
        Cow::Owned(regex::escape(query))
    };

    let mut builder = RegexBuilder::new(&pattern);
    builder.case_insensitive(!options.case_sensitive);
    if options.regex {
        // Match VS Code's Find bar: ^ and $ match line boundaries by default,
        // while `.` stays within a single line.
        builder.multi_line(true).crlf(true);
    }

    builder
        .build()
        .map(CompiledMatcher::Regex)
        .map_err(|err| format_regex_error(&err))
}

fn collect_matches(
    matcher: &CompiledMatcher,
    options: SearchOptions,
    text: &str,
    limit: usize,
) -> (Vec<Range<usize>>, bool) {
    let mut results = Vec::new();
    let mut truncated = false;

    let mut push_if_valid = |range: Range<usize>| -> bool {
        if options.whole_word && !is_whole_word(text, &range) {
            return true;
        }
        if results.len() >= limit {
            truncated = true;
            return false;
        }
        results.push(range);
        true
    };

    match matcher {
        CompiledMatcher::LiteralExact(needle) => {
            for (ix, found) in text.match_indices(needle) {
                if !push_if_valid(ix..ix + found.len()) {
                    break;
                }
            }
        }
        CompiledMatcher::Regex(re) => {
            for found in re.find_iter(text) {
                if !push_if_valid(found.start()..found.end()) {
                    break;
                }
            }
        }
    }

    (results, truncated)
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Checks VS Code-style whole-word boundaries: a boundary is only required on a
/// side of the match whose edge character is itself a word character (so queries
/// like `c++` or `(foo)` still match when bounded on their word-character edges).
fn is_whole_word(text: &str, range: &Range<usize>) -> bool {
    let Some(matched) = text.get(range.clone()) else {
        return false;
    };
    let prev_is_word = text[..range.start]
        .chars()
        .next_back()
        .is_some_and(is_word_char);
    let next_is_word = text[range.end..].chars().next().is_some_and(is_word_char);

    let mut chars = matched.chars();
    let Some(first) = chars.next() else {
        return prev_is_word != next_is_word;
    };
    let last = chars.next_back().unwrap_or(first);

    (!is_word_char(first) || !prev_is_word) && (!is_word_char(last) || !next_is_word)
}

fn format_regex_error(err: &regex::Error) -> String {
    match err {
        regex::Error::Syntax(msg) => msg
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map_or("invalid regular expression", |line| {
                line.strip_prefix("error: ").unwrap_or(line)
            })
            .to_string(),
        regex::Error::CompiledTooBig(_) => "regular expression is too large".to_string(),
        _ => err.to_string(),
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
    fn test_search_query_refinement() {
        let mut search = DocumentSearch::new();
        let rendered = "Alpha Beta Gamma Delta";

        search.update_query("a", rendered);
        assert_eq!(search.total_matches(), 6);

        search.update_query("al", rendered);
        assert_eq!(search.total_matches(), 1);

        search.update_query("alpha", rendered);
        assert_eq!(search.total_matches(), 1);
    }

    #[test]
    fn test_case_sensitive_matching() {
        let mut search = DocumentSearch::new();
        let rendered = "Alpha alpha ALPHA Café café";

        search.update_query("alpha", rendered);
        assert_eq!(search.total_matches(), 3);

        search.toggle_option(SearchOption::CaseSensitive, rendered);
        assert!(search.options().case_sensitive);
        assert_eq!(search.total_matches(), 1);
        assert_eq!(search.current_match(), Some(&(6..11)));

        search.update_query("Café", rendered);
        assert_eq!(search.total_matches(), 1);
        assert_eq!(search.current_match(), Some(&(18..23)));
    }

    #[test]
    fn test_whole_word_matching_including_punctuation_edges() {
        let mut search = DocumentSearch::new();
        let rendered = "foo bar foobar _foo foo_ (foo) foo-bar c++ objc++ c++11";

        search.set_options(
            SearchOptions {
                whole_word: true,
                ..Default::default()
            },
            rendered,
        );

        search.update_query("foo", rendered);
        // Matches: "foo" (0..3), "(foo)" (26..29), "foo-bar" (31..34).
        // Excludes: "foobar", "_foo", "foo_".
        assert_eq!(search.matches(), &[0..3, 26..29, 31..34]);

        // Leading word char + trailing non-word chars: matches standalone "c++" and "c++11",
        // rejects "objc++".
        search.update_query("c++", rendered);
        assert_eq!(search.matches(), &[39..42, 50..53]);

        // Both edges non-word chars: still matches "(foo)".
        search.update_query("(foo)", rendered);
        assert_eq!(search.total_matches(), 1);
        assert_eq!(search.current_match(), Some(&(25..30)));
    }

    #[test]
    fn test_regex_matching_multiline_case_and_whole_word_combinations() {
        let mut search = DocumentSearch::new();
        let rendered = "Item 12\nitem 345\nfoo12\nfoo bar";

        search.set_options(
            SearchOptions {
                regex: true,
                ..Default::default()
            },
            rendered,
        );

        // Multiline ^ and case-insensitive by default.
        search.update_query(r"^item \d+", rendered);
        assert_eq!(search.matches(), &[0..7, 8..16]);

        // Combine regex + case_sensitive.
        search.toggle_option(SearchOption::CaseSensitive, rendered);
        assert_eq!(search.total_matches(), 1);
        assert_eq!(search.current_match(), Some(&(8..16)));

        // Combine regex + whole_word (should reject "foo" inside "foo12", match "foo" in "foo bar").
        search.set_options(
            SearchOptions {
                regex: true,
                whole_word: true,
                case_sensitive: true,
            },
            rendered,
        );
        search.update_query(r"foo|item", rendered);
        assert_eq!(search.matches(), &[8..12, 23..26]);
    }

    #[test]
    fn test_invalid_regex_reports_error_and_recovers() {
        let mut search = DocumentSearch::new();
        let rendered = "hello [world]";

        search.set_options(
            SearchOptions {
                regex: true,
                ..Default::default()
            },
            rendered,
        );
        search.update_query("[unclosed", rendered);
        assert_eq!(search.total_matches(), 0);
        assert!(search.error().is_some());
        assert_eq!(search.match_display(), "Invalid regex");

        // Fixing the regex or turning off regex mode clears the error and finds matches.
        search.toggle_option(SearchOption::Regex, "hello [unclosed world");
        assert!(search.error().is_none());
        assert_eq!(search.total_matches(), 1);
        assert_eq!(search.match_display(), "1/1");
    }

    #[test]
    fn test_clear_query_preserves_search_options() {
        let mut search = DocumentSearch::new();
        search.set_options(
            SearchOptions {
                case_sensitive: true,
                whole_word: true,
                regex: true,
            },
            "alpha",
        );
        search.update_query("alpha", "alpha");
        assert_eq!(search.total_matches(), 1);

        search.clear_query();
        assert_eq!(search.total_matches(), 0);
        assert!(search.error().is_none());
        assert_eq!(
            search.options(),
            SearchOptions {
                case_sensitive: true,
                whole_word: true,
                regex: true,
            }
        );
    }

    #[test]
    fn test_toggling_option_preserves_nearest_match_offset() {
        let mut search = DocumentSearch::new();
        let rendered = "alpha Alpha alpha";

        search.update_query("alpha", rendered);
        assert_eq!(search.total_matches(), 3);
        search.next_match(); // now on "Alpha" at 6..11
        search.next_match(); // now on "alpha" at 12..17
        assert_eq!(search.current_match(), Some(&(12..17)));

        // Toggling case-sensitive drops "Alpha" at 6..11, leaving 0..5 and 12..17;
        // nearest match to offset 12 should remain 12..17 (index 1).
        search.toggle_option(SearchOption::CaseSensitive, rendered);
        assert_eq!(search.current_match(), Some(&(12..17)));
        assert_eq!(search.current_match_index(), 1);
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

        // Multi-byte unicode characters adjacent to matches
        let text_with_multibyte = "a・b and ab";
        search.update_query("a", text_with_multibyte);
        assert_eq!(search.total_matches(), 3);
        search.update_query("ab", text_with_multibyte);
        assert_eq!(search.total_matches(), 1);
        assert_eq!(search.current_match(), Some(&(10..12)));

        // Characters whose lowercase changes UTF-8 byte length (İ -> i\u{307}) must still yield
        // exact byte offsets in the original text.
        let turkish = "İstanbul test";
        search.update_query("test", turkish);
        assert_eq!(search.current_match(), Some(&(10..14)));
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

        let (ix, range) =
            search.find_block_for_target(&block_spans, &target_range, 100, 10, mapper);
        assert_eq!(ix, 5);
        assert!(range.is_none());
    }
}
