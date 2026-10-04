#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuzzyMatchResult {
    pub matched: bool,
    /// Lower is better; only meaningful when `matched` is true.
    pub score: f64,
}

/// Subsequence fuzzy match, case-insensitive (fzf/Quick Open style): every character of `query`
/// must appear in `text` in order, not necessarily contiguously. An empty query matches everything.
/// Score rewards a tighter, earlier match (shorter span, then earlier start) — lower is better.
pub fn fuzzy_match(query: &str, text: &str) -> FuzzyMatchResult {
    if query.is_empty() {
        return FuzzyMatchResult {
            matched: true,
            score: 0.0,
        };
    }

    let q: Vec<char> = query.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut qi = 0;
    let mut start: Option<usize> = None;
    let mut end = 0;

    for (ti, ch) in t.iter().enumerate() {
        if qi >= q.len() {
            break;
        }
        if *ch == q[qi] {
            start.get_or_insert(ti);
            end = ti;
            qi += 1;
        }
    }

    match start {
        Some(start) if qi == q.len() => {
            let span = end - start + 1;
            FuzzyMatchResult {
                matched: true,
                score: (span * 1000 + start) as f64,
            }
        }
        _ => FuzzyMatchResult {
            matched: false,
            score: f64::INFINITY,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matched(q: &str, t: &str) -> bool {
        fuzzy_match(q, t).matched
    }

    #[test]
    fn matches_a_contiguous_substring() {
        assert!(matched("fix", "fix the login bug"));
    }

    #[test]
    fn matches_non_contiguous_characters_in_order() {
        assert!(matched("fnc", "function"));
    }

    #[test]
    fn rejects_characters_out_of_order_or_missing() {
        assert!(!matched("cnf", "function"));
        assert!(!matched("fnz", "function"));
    }

    #[test]
    fn is_case_insensitive() {
        assert!(matched("FIX", "fix the login bug"));
        assert!(matched("fix", "FIX THE LOGIN BUG"));
    }

    #[test]
    fn empty_query_matches_everything_with_the_best_score() {
        let r = fuzzy_match("", "anything at all");
        assert!(r.matched);
        assert_eq!(r.score, 0.0);
    }

    #[test]
    fn scores_a_tighter_earlier_match_better() {
        let tight = fuzzy_match("fn", "function");
        let loose = fuzzy_match("fn", "far from concrete");
        assert!(tight.matched && loose.matched);
        assert!(tight.score < loose.score);
    }

    #[test]
    fn prefers_an_earlier_match_when_spans_are_equal() {
        let early = fuzzy_match("ab", "ab----------");
        let late = fuzzy_match("ab", "----------ab");
        assert!(early.score < late.score);
    }

    #[test]
    fn counts_characters_not_bytes() {
        assert_eq!(fuzzy_match("b", "\u{e9}b").score, 1001.0);
    }
}
