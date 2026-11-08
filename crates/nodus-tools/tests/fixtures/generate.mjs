// Writes tools.json beside this file from the TypeScript itself, so the Rust
// tests check the board tools' MCP answers byte for byte against 0.9.4's,
// on macOS and on Windows. Every id here is made up.
//
// Run from the repo root with Node 22:
//
//   node --experimental-strip-types crates/nodus-tools/tests/fixtures/generate.mjs
//
// It copies src/main/boardTools.ts and what it imports to a temporary folder,
// swapping only Electron's app and the store for stubs.
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const root = path.resolve(here, '../../../..')

function copy(work, from, name, replacements = []) {
  let source = fs.readFileSync(path.join(root, from), 'utf8')
  for (const [find, to] of replacements) {
    if (!source.includes(find)) throw new Error(`${from} no longer has: ${find}`)
    source = source.replace(find, to)
  }
  fs.writeFileSync(path.join(work, `${name}.mts`), source)
}

/** boardTools.ts as it loads on `platform`. */
async function load(platform) {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), `nodus-tools-${platform}-`))
  copy(work, 'src/shared/types.ts', 'types')
  copy(work, 'src/main/hostProtocol.ts', 'hostProtocol')
  copy(work, 'src/main/viewPage.ts', 'viewPage', [
    [`process.platform === 'win32'`, `${JSON.stringify(platform)} === 'win32'`]
  ])
  copy(work, 'src/main/boardTools.ts', 'boardTools', [
    [`import { app } from 'electron'`, `const app = { getVersion: () => '9.9.9' }`],
    [`from '@shared/types'`, `from './types.mts'`],
    [`from './hostProtocol'`, `from './hostProtocol.mts'`],
    [
      `import { projectIdForSession } from './store'`,
      `const projectIdForSession = (id: string) => (id === 'gone' ? null : 'p1')`
    ],
    [`from './viewPage'`, `from './viewPage.mts'`]
  ])
  const tools = await import(pathToFileURL(path.join(work, 'boardTools.mts')).href)
  const page = await import(pathToFileURL(path.join(work, 'viewPage.mts')).href)
  return { tools, page }
}

/** A value with what JSON.stringify writes in its own way: big and fractional numbers, -0, escapes, empties. */
const AWKWARD = {
  at: 1_700_000_000_000,
  half: 0.5,
  tiny: 1e-7,
  huge: 1e21,
  negativeZero: -0,
  text: 'quote " backslash \\ tab \t newline \n control \u0001 del \u007f é   😀',
  empty: {},
  none: [],
  nested: [{ a: [1, [2, {}]] }],
  nil: null
}

const RUN = 'run-session'
const handlers = {
  isRun: (id) => id === RUN,
  listProjects: () => AWKWARD,
  createProject: () => ({ projectId: 'p3' }),
  findProject: () => 'p1',
  listSessions: () => [],
  archiveSessions: () => ({ archived: [], removed: [], skipped: [] }),
  archiveSelf: () => ({ archiving: 'This session moves to the archive when this turn ends.' }),
  listNotes: () => [],
  createNote: () => ({ noteId: 'n1' }),
  updateNote: () => ({ noteId: 'n1' }),
  listViews: () => [],
  getView: () => ({ id: 'v1' }),
  createView: async () => ({ view_id: 'v1' }),
  updateView: async () => ({ view_id: 'v1', version: 2 })
}

const out = {}
for (const platform of ['darwin', 'win32']) {
  const { tools, page } = await load(platform)
  tools.configureBoardTools(handlers)
  const answer = async (sessionId, message) => JSON.stringify(await tools.answerMcp(sessionId, message))
  out[platform] = {
    viewGuide: page.VIEW_GUIDE,
    initialize: await answer('s1', { jsonrpc: '2.0', id: 1, method: 'initialize', params: {} }),
    initializeAsked: await answer('s1', { id: 'one', method: 'initialize', params: { protocolVersion: '2025-03-26' } }),
    toolsListSession: await answer('s1', { id: 2, method: 'tools/list' }),
    toolsListRun: await answer(RUN, { id: 3, method: 'tools/list' }),
    noId: await answer('s1', { method: 'ping' }),
    nullId: await answer('s1', { id: null, method: 'ping' }),
    unknown: await answer('s1', { id: 4, method: 'resources/list' }),
    pretty: await answer('s1', { id: 5, method: 'tools/call', params: { name: 'list_projects' } }),
    unknownTool: await answer('s1', { id: 6, method: 'tools/call', params: {} }),
    gone: await answer('gone', { id: 7, method: 'tools/call', params: { name: 'list_notes', arguments: [] } })
  }
  out.mcpConfig = tools.BOARD_MCP_CONFIG
  out.toolRules = tools.BOARD_TOOL_RULES
}

fs.writeFileSync(path.join(here, 'tools.json'), JSON.stringify(out, null, 2) + '\n')
console.log('wrote tools.json')
