import { describe, expect, it } from 'vitest'
import { parseDiff } from './diff'

const header = ['diff --git a/a.ts b/a.ts', 'index 1111111..2222222 100644', '--- a/a.ts', '+++ b/a.ts']

describe('parseDiff', () => {
  it('numbers lines on each side', () => {
    const { lines, maxLineNo } = parseDiff(
      [...header, '@@ -3,3 +3,4 @@ function f() {', ' keep', '-old', '+new', '+added', ' end', ''].join('\n')
    )
    expect(lines.map(({ kind, text, oldNo, newNo }) => ({ kind, text, oldNo, newNo }))).toEqual([
      { kind: 'hunk', text: '@@ -3,3 +3,4 @@ function f() {', oldNo: undefined, newNo: undefined },
      { kind: 'context', text: 'keep', oldNo: 3, newNo: 3 },
      { kind: 'del', text: 'old', oldNo: 4, newNo: undefined },
      { kind: 'add', text: 'new', oldNo: undefined, newNo: 4 },
      { kind: 'add', text: 'added', oldNo: undefined, newNo: 5 },
      { kind: 'context', text: 'end', oldNo: 5, newNo: 6 }
    ])
    expect(maxLineNo).toBe(6)
  })

  it('marks the changed part of a line that pairs with another', () => {
    const { lines } = parseDiff([...header, '@@ -1 +1 @@', '-const a = 1', '+const a = 2'].join('\n'))
    expect(lines[1].changed).toEqual([10, 11])
    expect(lines[2].changed).toEqual([10, 11])
  })

  it('marks nothing when the blocks differ in length or share nothing', () => {
    const uneven = parseDiff([...header, '@@ -1,2 +1 @@', '-one', '-two', '+three'].join('\n'))
    expect(uneven.lines.some((l) => l.changed)).toBe(false)
    const unrelated = parseDiff([...header, '@@ -1 +1 @@', '-abc', '+xyz'].join('\n'))
    expect(unrelated.lines.some((l) => l.changed)).toBe(false)
  })

  it('pairs lines across a missing-newline note', () => {
    const { lines } = parseDiff(
      [...header, '@@ -1 +1 @@', '-let x = 1', '\\ No newline at end of file', '+let x = 2'].join('\n')
    )
    expect(lines[2]).toEqual({ kind: 'note', text: 'No newline at end of file' })
    expect(lines[1].changed).toEqual([8, 9])
    expect(lines[3].changed).toEqual([8, 9])
  })

  it('reads a blank context line that lost its space', () => {
    const { lines } = parseDiff([...header, '@@ -1,2 +1,2 @@', '', ' b'].join('\n'))
    expect(lines[1]).toEqual({ kind: 'context', text: '', oldNo: 1, newNo: 1 })
  })

  it('sums up a diff with no hunks', () => {
    const summary = (...extra: string[]) => parseDiff(['diff --git a/a b/b', ...extra].join('\n')).summary
    expect(summary('similarity index 100%', 'rename from a.ts', 'rename to b.ts')).toBe('Renamed from a.ts without changes')
    expect(summary('old mode 100644', 'new mode 100755')).toBe('File mode changed from 100644 to 100755')
    expect(summary('new file mode 100644')).toBe('Empty file')
    expect(parseDiff('').summary).toBe('No changes to show')
    expect(parseDiff([...header, '@@ -1 +1 @@', '-a', '+b'].join('\n')).summary).toBeUndefined()
  })
})
