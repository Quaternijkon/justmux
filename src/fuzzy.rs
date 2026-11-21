//! Unicode matching shared by command and resource pickers.
use std::cmp::Reverse;

use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{Atom, AtomKind, CaseMatching, Normalization},
};
use unicode_segmentation::UnicodeSegmentation;

/// Literal matches precede fuzzy matches in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    Exact,
    Prefix,
    Substring,
    Fuzzy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub kind: MatchKind,
    /// Higher scores rank first within the same kind.
    pub score: u32,
    /// Sorted, unique indices into the label's extended grapheme clusters.
    pub indices: Vec<usize>,
}

impl FuzzyMatch {
    pub fn sort_key(&self) -> (MatchKind, Reverse<u32>) {
        (self.kind, Reverse(self.score))
    }
}

/// Retains nucleo's scratch allocation across the candidates in a search.
#[derive(Debug, Clone)]
pub struct FuzzyMatcher {
    matcher: Matcher,
    haystack: Vec<char>,
    indices: Vec<u32>,
}

impl Default for FuzzyMatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl FuzzyMatcher {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            haystack: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Every whitespace-separated term must match, in any order. Matching is
    /// case-insensitive, including queries containing uppercase letters.
    pub fn match_one(&mut self, query: &str, candidate: &str) -> Option<FuzzyMatch> {
        let query = query.trim();
        if query.is_empty() {
            return Some(FuzzyMatch {
                kind: MatchKind::Fuzzy,
                score: 0,
                indices: Vec::new(),
            });
        }

        // One codepoint per grapheme keeps nucleo's indices aligned with the
        // original label, including combining marks and joined emoji.
        self.haystack.clear();
        self.haystack
            .extend(nucleo_matcher::chars::graphemes(candidate));
        self.indices.clear();
        let haystack = Utf32Str::Unicode(&self.haystack);
        let mut score = 0u32;
        for term in query.split_whitespace() {
            let atom = Atom::new(
                term,
                CaseMatching::Ignore,
                Normalization::Smart,
                AtomKind::Fuzzy,
                false,
            );
            score = score.saturating_add(u32::from(atom.indices(
                haystack,
                &mut self.matcher,
                &mut self.indices,
            )?));
        }

        let (kind, indices) = match literal_match(query, candidate) {
            Some((kind, indices)) => (kind, indices),
            None => {
                self.indices.sort_unstable();
                self.indices.dedup();
                (
                    MatchKind::Fuzzy,
                    self.indices.iter().map(|&index| index as usize).collect(),
                )
            }
        };
        Some(FuzzyMatch {
            kind,
            score,
            indices,
        })
    }
}

fn literal_match(query: &str, candidate: &str) -> Option<(MatchKind, Vec<usize>)> {
    let needle: Vec<_> = query.graphemes(true).map(str::to_lowercase).collect();
    let haystack: Vec<_> = candidate.graphemes(true).map(str::to_lowercase).collect();
    if needle.len() > haystack.len() {
        return None;
    }
    let start = haystack
        .windows(needle.len())
        .position(|window| window == needle)?;
    let kind = if needle.len() == haystack.len() {
        MatchKind::Exact
    } else if start == 0 {
        MatchKind::Prefix
    } else {
        MatchKind::Substring
    };
    Some((kind, (start..start + needle.len()).collect()))
}

/// Compatibility score for callers using ascending numeric ordering.
/// New callers can retain a matcher and sort by `FuzzyMatch::sort_key`.
pub fn score(query: &str, candidate: &str) -> Option<usize> {
    let matched = FuzzyMatcher::new().match_one(query, candidate)?;
    if query.trim().is_empty() {
        return Some(0);
    }
    Some(
        matched.kind as usize * 65_536 + usize::from(u16::MAX) - matched.score.min(65_535) as usize,
    )
}

pub fn rank_strings<'a>(query: &str, values: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    rank_by(query, values, |value| value)
}

pub fn rank_by<T>(
    query: &str,
    values: impl IntoIterator<Item = T>,
    label: impl Fn(&T) -> &str,
) -> Vec<T> {
    let mut matcher = FuzzyMatcher::new();
    let mut ranked: Vec<_> = values
        .into_iter()
        .enumerate()
        .filter_map(|(index, value)| {
            matcher
                .match_one(query, label(&value))
                .map(|matched| (matched.sort_key(), index, value))
        })
        .collect();
    ranked.sort_by_key(|(key, index, _)| (*key, *index));
    ranked.into_iter().map(|(_, _, value)| value).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_tiers_rank_before_fuzzy_matches() {
        assert_eq!(
            rank_strings("api", ["a-proxy-interface", "my-api", "api-dev", "api"]),
            ["api", "api-dev", "my-api", "a-proxy-interface"]
        );
        assert!(score("api", "api") < score("api", "api-dev"));
    }

    #[test]
    fn empty_and_equal_matches_preserve_caller_order() {
        assert_eq!(rank_strings("", ["b", "a"]), ["b", "a"]);
        assert_eq!(rank_strings("  ", ["b", "a"]), ["b", "a"]);
        assert_eq!(rank_strings("api", ["API", "api"]), ["API", "api"]);
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn terms_match_in_any_order_and_all_are_required() {
        let mut matcher = FuzzyMatcher::new();
        let matched = matcher.match_one("proj my", "my project").expect("match");
        assert_eq!(matched.indices, [0, 1, 3, 4, 5, 6]);
        assert!(matcher.match_one("my missing", "my project").is_none());
        assert!(matcher.match_one("WORK", "work").is_some());
        assert!(matcher.match_one("xyz", "example").is_none());
    }

    #[test]
    fn unicode_highlights_use_grapheme_indices() {
        let mut matcher = FuzzyMatcher::new();
        assert_eq!(
            matcher.match_one("项β", "项目-β").expect("CJK").indices,
            [0, 3]
        );
        assert_eq!(
            matcher
                .match_one("bar", "👩‍👩‍👧 cafe\u{301} bar")
                .expect("emoji prefix")
                .indices,
            [7, 8, 9]
        );
        assert_eq!(
            matcher
                .match_one("cafe", "cafe\u{301} bar")
                .expect("combining accent")
                .indices,
            [0, 1, 2, 3]
        );
        assert_eq!(
            matcher
                .match_one("É", "équipe")
                .expect("uppercase accent")
                .indices,
            [0]
        );
    }
}
