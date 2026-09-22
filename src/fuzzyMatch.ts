export interface FuzzyMatchResult {
  matched: boolean;
  /** Lower is better; only meaningful when `matched` is true. */
  score: number;
}

/**
 * Subsequence fuzzy match, case-insensitive: every character of `query` must appear in `text` in
 * order, not necessarily contiguously — the standard "fuzzy find" style (fzf, VS Code's own Quick
 * Open, and the same convention agent-deck's own `/` search uses). `"fnc"` matches `"function"`.
 * An empty query matches everything with the best possible score, so callers don't need a special
 * case for "no query typed yet".
 *
 * Score rewards a tighter, earlier match: the span from the first matched character to the last
 * (shorter is better), with the start position as a tiebreaker (earlier is better) — so `"fn"`
 * scores better against `"function"` than against `"far from concrete"`, even though both match.
 * Deliberately not a more elaborate scoring model (consecutive-run bonuses, word-boundary
 * weighting, etc.) — this is meant to rank a QuickPick list sensibly, not reproduce fzf's own
 * algorithm exactly.
 */
export function fuzzyMatch(query: string, text: string): FuzzyMatchResult {
  if (!query) {
    return { matched: true, score: 0 };
  }

  const q = query.toLowerCase();
  const t = text.toLowerCase();
  let qi = 0;
  let start = -1;
  let end = -1;

  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) {
      if (start === -1) {
        start = ti;
      }
      end = ti;
      qi++;
    }
  }

  if (qi < q.length) {
    return { matched: false, score: Infinity };
  }

  const span = end - start + 1;
  return { matched: true, score: span * 1000 + start };
}
