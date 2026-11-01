// How a session search reads its query, shared by the search in the main
// process and the highlighting in the renderer.

/** More words than this add little but cost a pass over every message each. */
const MAX_TERMS = 8

/** The words of a query, lowercased, each once; longest first so highlights prefer them. */
export function queryTerms(query: string): string[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  return [...new Set(words)].slice(0, MAX_TERMS).sort((a, b) => b.length - a.length)
}

function escape(term: string) {
  return term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/** Matches any of the terms, ignoring case. */
export function termsPattern(terms: string[]): RegExp | null {
  return terms.length ? new RegExp(terms.map(escape).join('|'), 'giu') : null
}

/** Where the terms occur in `text`, as sorted [start, end) ranges with overlaps merged. */
export function matchRanges(text: string, terms: string[]): [number, number][] {
  const re = termsPattern(terms)
  if (!re) return []
  const ranges: [number, number][] = []
  for (const m of text.matchAll(re)) {
    const start = m.index
    const end = start + m[0].length
    const last = ranges[ranges.length - 1]
    if (last && start <= last[1]) last[1] = Math.max(last[1], end)
    else ranges.push([start, end])
  }
  return ranges
}
