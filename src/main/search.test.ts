import fs from 'fs'
import os from 'os'
import path from 'path'
import { afterAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { matchRanges, queryTerms } from '@shared/search'
import { transcriptDir } from './discovery'
import { clearSearchIndex, searchSessions } from './search'

// Transcripts go under a home of their own.
const home = fs.mkdtempSync(path.join(os.tmpdir(), 'nodus-search-'))
vi.spyOn(os, 'homedir').mockReturnValue(home)
const repo = path.join(home, 'repo')
fs.mkdirSync(repo)

afterAll(() => fs.rmSync(home, { recursive: true, force: true }))

let clock = Date.parse('2026-11-01T10:00:00Z')
const at = () => new Date((clock += 1000)).toISOString()

const prompt = (text: string) => ({ type: 'user', timestamp: at(), message: { role: 'user', content: text } })
const reply = (text: string) => ({
  type: 'assistant',
  timestamp: at(),
  message: { id: `m${clock}`, role: 'assistant', content: [{ type: 'text', text }] }
})
const tool = (name: string, input: Record<string, unknown>) => ({
  type: 'assistant',
  timestamp: at(),
  message: { id: `m${clock}`, role: 'assistant', content: [{ type: 'tool_use', id: `t${clock}`, name, input }] }
})
const result = (content: string) => ({
  type: 'user',
  timestamp: at(),
  message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: 't', content }] }
})

function write(sessionId: string, lines: object[], file = path.join(transcriptDir(repo), `${sessionId}.jsonl`)) {
  fs.mkdirSync(path.dirname(file), { recursive: true })
  fs.writeFileSync(file, lines.map((l) => JSON.stringify(l)).join('\n') + '\n')
}

const ids = {
  auth: '00000000-0000-4000-8000-000000000001',
  docs: '00000000-0000-4000-8000-000000000002',
  agent: '00000000-0000-4000-8000-000000000003'
}
const targets = [
  { sessionId: ids.auth, title: 'Fix login' },
  { sessionId: ids.docs, title: 'Write the README' },
  { sessionId: ids.agent, title: 'Tidy up' }
]
const search = (query: string) => searchSessions(repo, targets, query)

beforeEach(() => {
  clearSearchIndex()
  write(ids.auth, [
    prompt('The login redirect loops. Check the auth middleware please'),
    reply('The session cookie is read before it is set.'),
    tool('Edit', {
      file_path: path.join(repo, 'src/middleware/auth.ts'),
      old_string: 'const token = req.cookies.session',
      new_string: 'const token = req.signedCookies.session'
    }),
    result('export function authMiddleware() { /* the whole file, which mentions README */ }'),
    reply('Fixed: the middleware now reads the signed cookie.')
  ])
  write(ids.docs, [
    prompt('Write a README for the project'),
    tool('Write', { file_path: path.join(repo, 'README.md'), content: '# Project\n\nAuth is handled elsewhere.' }),
    reply('Added the README.')
  ])
  write(ids.agent, [prompt('Tidy up the repo'), reply('I asked an agent to look around.')])
  write(
    ids.agent,
    [
      { ...prompt('Look for dead code (the session’s prompt to its agent)'), isSidechain: true },
      { ...tool('Bash', { command: 'rg -l signedCookies src' }), isSidechain: true }
    ],
    path.join(transcriptDir(repo), ids.agent, 'subagents', 'agent-a1.jsonl')
  )
})

describe('searchSessions', () => {
  it('finds the session whose transcript mentions every word, path matches included', async () => {
    const hits = await search('auth middleware')
    expect(hits.map((h) => h.sessionId)).toEqual([ids.auth])
    expect(hits[0].matches).toBe(2)
    expect(hits[0].snippets[0]).toMatchObject({ kind: 'prompt', text: expect.stringContaining('auth middleware') })
    expect(hits[0].snippets[1]).toMatchObject({ kind: 'tool', tool: 'Edit', text: 'src/middleware/auth.ts' })
  })

  it('names the file when the match is in what a tool wrote', async () => {
    const [hit] = await search('signedCookies')
    expect(hit.sessionId).toBe(ids.auth)
    const edit = hit.snippets.find((s) => s.kind === 'tool' && !s.agent)
    expect(edit).toMatchObject({ tool: 'Edit', file: 'auth.ts', text: expect.stringContaining('req.signedCookies.session') })
    expect(edit!.text).not.toContain('src/middleware')
  })

  it('searches subagents’ work, but not the prompt the session gave them twice', async () => {
    const hits = await search('signedCookies')
    const agent = hits.find((h) => h.sessionId === ids.agent)
    expect(agent?.snippets).toEqual([expect.objectContaining({ kind: 'tool', tool: 'Bash', agent: true })])
    expect(await search('dead code')).toEqual([])
  })

  it('leaves tool output out', async () => {
    const hits = await search('authMiddleware')
    expect(hits).toEqual([])
    expect((await search('readme')).map((h) => h.sessionId)).toEqual([ids.docs])
  })

  it('needs every word somewhere in the session or its title', async () => {
    expect(await search('auth readme')).toEqual([
      expect.objectContaining({ sessionId: ids.docs }) // "Auth is handled elsewhere" in the README it wrote
    ])
    const titled = await search('login cookie')
    expect(titled.map((h) => h.sessionId)).toEqual([ids.auth])
  })

  it('ranks the whole phrase above scattered words', async () => {
    write(ids.docs, [prompt('Middleware for auth, maybe later'), reply('Noted.')])
    const hits = await search('auth middleware')
    expect(hits.map((h) => h.sessionId)).toEqual([ids.auth, ids.docs])
  })

  it('returns title-only matches without snippets', async () => {
    const hits = await search('tidy')
    expect(hits[0]).toMatchObject({ sessionId: ids.agent })
    const titleOnly = await searchSessions(repo, [{ sessionId: '00000000-0000-4000-8000-00000000000f', title: 'Tidy notes' }], 'tidy')
    expect(titleOnly).toEqual([{ sessionId: '00000000-0000-4000-8000-00000000000f', matches: 0, snippets: [] }])
  })

  it('reads what a live session adds to its transcript', async () => {
    expect(await search('websocket')).toEqual([])
    const file = path.join(transcriptDir(repo), `${ids.docs}.jsonl`)
    fs.appendFileSync(file, JSON.stringify(prompt('Now add a websocket endpoint')) + '\n')
    expect((await search('websocket')).map((h) => h.sessionId)).toEqual([ids.docs])
  })

  it('cuts long text down to a line around the match', async () => {
    const long = `${'word '.repeat(100)}the needle is here ${'more '.repeat(100)}`
    write(ids.docs, [prompt(long)])
    const [hit] = await search('needle')
    const { text, ranges } = hit.snippets[0]
    expect(text.startsWith('…') && text.endsWith('…')).toBe(true)
    expect(text.length).toBeLessThan(180)
    expect(ranges.map(([s, e]) => text.slice(s, e))).toEqual(['needle'])
  })

  it('returns nothing for an empty query', async () => {
    expect(await search('   ')).toEqual([])
  })
})

describe('query helpers', () => {
  it('splits, lowercases and dedupes words, longest first', () => {
    expect(queryTerms('  Auth  middleware AUTH ')).toEqual(['middleware', 'auth'])
  })

  it('merges overlapping highlights and ignores case', () => {
    expect(matchRanges('Auth middleware: authmiddleware', ['middleware', 'auth'])).toEqual([
      [0, 4],
      [5, 15],
      [17, 31]
    ])
  })

  it('treats regex characters literally', () => {
    expect(matchRanges('call foo.bar() now', ['foo.bar()'])).toEqual([[5, 14]])
  })
})
