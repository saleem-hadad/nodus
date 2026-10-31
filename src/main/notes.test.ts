import { beforeEach, describe, expect, it, vi } from 'vitest'
import { CARD_H, CARD_W, GAP, NOTE_H, NOTE_W } from '@shared/geometry'
import { NOTE_MAX_CHARS, createNote, listNotes, updateNote } from './notes'
import { getBoard } from './store'

vi.mock('electron', () => ({ app: {} }))
// The real store, apart from writing state.json.
vi.mock('./store', async (importOriginal) => ({ ...(await importOriginal<typeof import('./store')>()), save: vi.fn() }))

// Each test gets its own project's board, with one card at (2000, 0).
let project: string
let next = 0

beforeEach(() => {
  project = `project-${++next}`
  getBoard(project).nodes.s1 = { sessionId: 's1', x: 2000, y: 0, archived: false, createdAt: 0 }
})

describe('createNote', () => {
  it('puts a session’s first note beside its card', () => {
    const note = createNote(project, 's1', { text: '  Findings  ' })
    expect(note).toMatchObject({ text: 'Findings', color: 'yellow', from: 's1', x: 2000 + CARD_W + GAP, y: 0 })
    expect(getBoard(project).notes![note.id]).toBe(note)
  })

  it('puts the next one under the last', () => {
    const first = createNote(project, 's1', { text: 'One' })
    const second = createNote(project, 's1', { text: 'Two', color: 'green' })
    expect(second).toMatchObject({ x: first.x, y: first.y + NOTE_H + GAP, color: 'green' })
  })

  it('keeps clear of the board’s cards', () => {
    getBoard(project).nodes.s2 = { sessionId: 's2', x: 2000 + CARD_W + GAP, y: 0, archived: false, createdAt: 0 }
    const note = createNote(project, 's1', { text: 'Elsewhere' })
    const card = { x: 2000 + CARD_W + GAP, y: 0, w: CARD_W, h: CARD_H }
    const overlaps = note.x < card.x + card.w && note.x + NOTE_W > card.x && note.y < card.y + card.h && note.y + NOTE_H > card.y
    expect(overlaps).toBe(false)
  })

  it('stays square for short text and grows to a limit for long text', () => {
    const short = createNote(project, 's1', { text: 'Short' })
    expect(short.w).toBeUndefined()
    expect(short.h).toBeUndefined()
    const long = createNote(project, 's1', { text: 'word '.repeat(400) })
    expect(long).toMatchObject({ w: 360, h: 480 })
  })

  it('needs some text, and keeps no more than the limit', () => {
    expect(() => createNote(project, 's1', { text: '   ' })).toThrow('A note needs some text')
    expect(() => createNote(project, 's1', {})).toThrow('A note needs some text')
    expect(createNote(project, 's1', { text: 'x'.repeat(NOTE_MAX_CHARS + 50) }).text).toHaveLength(NOTE_MAX_CHARS)
  })
})

describe('updateNote', () => {
  it('changes the text and colour, and keeps the size', () => {
    const note = createNote(project, 's1', { text: 'Before' })
    note.w = 500
    updateNote(project, note.id, { text: ' After ', color: 'red' })
    expect(getBoard(project).notes![note.id]).toMatchObject({ text: 'After', color: 'red', w: 500 })
  })

  it('leaves out what the patch doesn’t say', () => {
    const note = createNote(project, 's1', { text: 'Keep', color: 'blue' })
    updateNote(project, note.id, {})
    expect(note).toMatchObject({ text: 'Keep', color: 'blue' })
    expect(() => updateNote(project, note.id, { text: ' ' })).toThrow('A note needs some text')
  })

  it('only finds the board’s own notes', () => {
    expect(() => updateNote(project, 'missing', { text: 'x' })).toThrow('There’s no note with that id on this board')
    for (const id of ['__proto__', 'constructor', 'toString', 'hasOwnProperty']) {
      expect(() => updateNote(project, id, { text: 'polluted', color: 'red' })).toThrow('There’s no note')
    }
    expect(({} as { text?: string }).text).toBeUndefined()
    expect(({} as { color?: string }).color).toBeUndefined()
  })
})

describe('listNotes', () => {
  it('lists notes oldest first and says which the session wrote', () => {
    const board = getBoard(project)
    board.notes!.user = { id: 'user', text: 'Mine', color: 'yellow', x: 0, y: 0, createdAt: 2 }
    board.notes!.claude = { id: 'claude', text: 'Claude’s', color: 'blue', x: 0, y: 0, from: 's1', createdAt: 1 }
    expect(listNotes(project, 's1')).toEqual([
      { id: 'claude', text: 'Claude’s', color: 'blue', writtenByYou: true },
      { id: 'user', text: 'Mine', color: 'yellow', writtenByYou: undefined }
    ])
  })
})
