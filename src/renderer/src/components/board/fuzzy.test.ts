import { describe, expect, it } from 'vitest'
import { fuzzyMatch } from './fuzzy'

const score = (query: string, text: string) => fuzzyMatch(query, text)?.score ?? -Infinity

describe('fuzzyMatch', () => {
  it('matches everything with an empty query', () => {
    expect(fuzzyMatch('  ', 'anything')).toEqual({ score: 0, indices: [] })
  })

  it('needs every character, in order', () => {
    expect(fuzzyMatch('xyz', 'Fix the board')).toBeNull()
    expect(fuzzyMatch('ba', 'abc')).toBeNull()
  })

  it('ignores case and spaces in the query', () => {
    expect(fuzzyMatch('F B', 'fix bug')?.indices).toEqual([0, 4])
  })

  it('lands on word starts, including camelCase ones', () => {
    expect(fuzzyMatch('bv', 'open BoardView')?.indices).toEqual([5, 10])
    expect(fuzzyMatch('sr', 'session-rail')?.indices).toEqual([0, 8])
  })

  it('ranks a run of characters above scattered ones', () => {
    expect(score('board', 'Board view')).toBeGreaterThan(score('board', 'big old area red dog'))
  })

  it('ranks a match at the start above one later on', () => {
    expect(score('fix', 'fix login')).toBeGreaterThan(score('fix', 'login fix'))
  })

  it('prefers the shorter title when they match alike', () => {
    expect(score('api', 'api')).toBeGreaterThan(score('api', 'api docs'))
  })
})
