export interface FuzzyMatchResult {
  matched: boolean;
  /** Lower is better; only meaningful when `matched` is true. */
  score: number;
}

/**
 * Subsequence fuzzy match, case-insensitive (fzf/Quick Open style): every character of `query`
 * must appear in `text` in order, not necessarily contiguously. An empty query matches everything.
 * Score rewards a tighter, earlier match (shorter span, then earlier start) — lower is better.
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
