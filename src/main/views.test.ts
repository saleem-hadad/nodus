import fs from 'fs'
import os from 'os'
import path from 'path'
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { CARD_W, GAP } from '@shared/geometry'
import type { BoardView } from '@shared/types'
import { getBoard, getState } from './store'
import {
  approveCommand,
  callView,
  configureViews,
  createViewFromTool,
  flushViews,
  getViewForTool,
  normalizeCommands,
  normalizeRefresh,
  removeView,
  reportView,
  runCommand,
  servePage,
  updateViewFromTool
} from './views'

vi.mock('electron', () => ({
  app: { getPath: () => process.env.NODUS_TEST_USER_DATA },
  shell: { openExternal: vi.fn() }
}))
// The real store, apart from writing state.json.
vi.mock('./store', async (importOriginal) => ({ ...(await importOriginal<typeof import('./store')>()), save: vi.fn() }))
vi.mock('./pty', () => ({
  spawnSession: vi.fn(async () => {}),
  isLive: vi.fn(() => false),
  write: vi.fn(),
  kill: vi.fn(async () => {})
}))
vi.mock('./hooks', () => ({ getStatus: vi.fn(() => 'done'), forgetStatus: vi.fn() }))
vi.mock('./board', () => ({ hasTranscript: () => false, sessionList: () => [{ id: 's1', title: 'Card' }] }))
vi.mock('./git', () => ({ gitStatus: async () => null }))
vi.mock('./env', () => ({
  spawnEnv: (extra: Record<string, string> = {}) => ({ PATH: process.env.PATH ?? '', ...extra }),
  userShell: () => '/bin/sh'
}))

const userData = fs.mkdtempSync(path.join(os.tmpdir(), 'nodus-views-'))
const notify = vi.fn()

let project: string
let next = 0

/** The board's newest view. */
const newest = (projectId: string) =>
  Object.values(getBoard(projectId).views!).sort((a, b) => b.createdAt - a.createdAt)[0]

beforeAll(() => {
  process.env.NODUS_TEST_USER_DATA = userData
  configureViews({
    notify,
    // The board loads a changed page as soon as it hears of it.
    boardChanged: (projectId) =>
      setTimeout(() => {
        const view = newest(projectId)
        if (view) reportView(projectId, view.id, { loaded: view.version })
      }, 0),
    envReady: Promise.resolve()
  })
})

afterAll(() => {
  flushViews()
  fs.rmSync(userData, { recursive: true, force: true })
})

// Each test gets its own project, with one card, s1, at (2000, 0).
beforeEach(() => {
  project = `project-${++next}`
  getState().projects.push({ id: project, name: project, repoPath: userData, color: 'blue', createdAt: 0 })
  getBoard(project).nodes.s1 = { sessionId: 's1', x: 2000, y: 0, archived: false, createdAt: 0 }
  notify.mockClear()
})

const html = '<!doctype html><html><head></head><body><p id="n"></p></body></html>'

describe('normalizeCommands', () => {
  it('keeps a command line the user allowed allowed, whatever it is called now', () => {
    const [cmd] = normalizeCommands([{ name: 'list', run: 'vercel ls --json', every: 10 }], { allowedRuns: ['vercel ls --json'] })
    expect(cmd).toEqual({ name: 'list', run: 'vercel ls --json', every: 10, approved: 'vercel ls --json', denied: undefined })
  })

  it('asks again for a command line that changed', () => {
    expect(normalizeCommands([{ name: 'deploys', run: 'vercel ls; rm -rf ~' }], { allowedRuns: ['vercel ls'] })[0].approved).toBeUndefined()
  })

  it('runs no more than every minute, nor less than weekly', () => {
    const cmds = normalizeCommands([
      { name: 'a', run: 'a', every: 0.2 },
      { name: 'b', run: 'b', every: 99999 },
      { name: 'c', run: 'c', every: -5 }
    ])
    expect(cmds.map((c) => c.every)).toEqual([1, 7 * 24 * 60, undefined])
  })

  it('turns down commands it can’t use, saying why', () => {
    expect(() => normalizeCommands('ls')).toThrow('commands must be a list')
    expect(() => normalizeCommands([{ name: 'x' }])).toThrow('needs a name and a run')
    expect(() => normalizeCommands([{ name: 'x', run: 'a' }, { name: 'x', run: 'b' }])).toThrow('two commands called “x”')
  })
})

describe('normalizeRefresh', () => {
  it('asks the brain at most every ten minutes', () => {
    expect(normalizeRefresh({ every: 1, prompt: ' Fetch my tickets ' })).toEqual({ every: 10, prompt: 'Fetch my tickets' })
    expect(normalizeRefresh(null)).toBeUndefined()
    expect(() => normalizeRefresh({ every: 60 })).toThrow('refresh needs every')
  })
})

describe('create_view and update_view', () => {
  it('puts a view up beside the card of the session that made it', async () => {
    const reply = await createViewFromTool(project, 's1', { title: 'Deploys', html, data: { rows: [] } })
    const view = getBoard(project).views![reply.view_id]
    expect(view).toMatchObject({ title: 'Deploys', description: 'Deploys', from: 's1', version: 1, x: 2000 + CARD_W + GAP, y: 0 })
    expect(reply).toMatchObject({ waiting_for_approval: [], errors: [] })
    expect(fs.readFileSync(path.join(userData, 'views', view.id, 'index.html'), 'utf8')).toBe(html)
  })

  it('reports the errors the page hit as it loaded', async () => {
    const created = createViewFromTool(project, 's1', { title: 'Broken', html })
    // The page fails while the tool waits on it.
    setTimeout(() => reportView(project, newest(project).id, { error: 'TypeError: rows is undefined' }), 5)
    expect((await created).errors).toEqual(['TypeError: rows is undefined'])
  })

  it('sends new data to the page in place, without reloading it', async () => {
    const { view_id } = await createViewFromTool(project, 's1', { title: 'Users', html })
    const reply = await updateViewFromTool(project, 's1', { view_id, data: { users: 42 } })
    expect(reply.version).toBe(1)
    expect(notify).toHaveBeenCalledWith(project, view_id, 'data', { users: 42 })
    expect(getViewForTool(project, 's1', view_id).data).toEqual({ users: 42 })
  })

  it('reloads the page when its html changes', async () => {
    const { view_id } = await createViewFromTool(project, 's1', { title: 'Users', html })
    const reply = await updateViewFromTool(project, 's1', { view_id, html: '<p>v2</p>' })
    expect(reply.version).toBe(2)
    expect(getViewForTool(project, 's1', view_id).html).toBe('<p>v2</p>')
  })

  it('lets a brain leave out its own view’s id, and nobody else', async () => {
    const { view_id } = await createViewFromTool(project, 's1', { title: 'Mine', html })
    getBoard(project).nodes.brain = { sessionId: 'brain', x: 0, y: 0, archived: false, viewOf: view_id, createdAt: 0 }
    await updateViewFromTool(project, 'brain', { title: 'Renamed' })
    expect(getBoard(project).views![view_id].title).toBe('Renamed')
    await expect(updateViewFromTool(project, 's1', { title: 'Nope' })).rejects.toThrow('view_id is required')
    await expect(updateViewFromTool(project, 's1', { view_id: '__proto__', title: 'x' })).rejects.toThrow('no view with that id')
  })

  it('needs a title and a page', async () => {
    await expect(createViewFromTool(project, 's1', { html })).rejects.toThrow('needs a title')
    await expect(createViewFromTool(project, 's1', { title: 'x', html: '  ' })).rejects.toThrow('html must be the page')
  })
})

describe('commands', () => {
  const withCommand = async (run: string) => {
    const { view_id } = await createViewFromTool(project, 's1', {
      title: 'Count',
      html,
      commands: [{ name: 'count', run }]
    })
    return view_id
  }

  it('waits for the user to allow a command before it runs', async () => {
    const viewId = await withCommand('echo 42')
    expect(getViewForTool(project, 's1', viewId).commands[0].allowed).toBe('waiting for the user')
    await expect(runCommand(project, viewId, 'count')).rejects.toThrow('waiting for the user to allow it')
  })

  it('runs an allowed command in the project folder, and sends the page its output', async () => {
    const viewId = await withCommand('printf "%s" "$(pwd)"')
    approveCommand(project, viewId, 'count', true)
    const result = await runCommand(project, viewId, 'count')
    expect(result).toMatchObject({ name: 'count', code: 0, stdout: fs.realpathSync(userData) })
    expect(notify).toHaveBeenCalledWith(project, viewId, 'result', result)
  })

  it('remembers a command line the user allowed when Claude puts it back', async () => {
    const viewId = await withCommand('echo one')
    approveCommand(project, viewId, 'count', true)
    await updateViewFromTool(project, 's1', { view_id: viewId, commands: [{ name: 'count', run: 'echo two' }] })
    expect(getViewForTool(project, 's1', viewId).commands[0].allowed).toBe('waiting for the user')
    await updateViewFromTool(project, 's1', { view_id: viewId, commands: [{ name: 'count', run: 'echo one' }] })
    expect(getViewForTool(project, 's1', viewId).commands[0].allowed).toBe('yes')
  })

  it('keeps a command the user turned down from running', async () => {
    const viewId = await withCommand('echo no')
    approveCommand(project, viewId, 'count', false)
    await expect(runCommand(project, viewId, 'count')).rejects.toThrow('didn’t allow')
  })

  it('reports a failing command’s exit code and what it printed', async () => {
    const viewId = await withCommand('echo oops >&2; exit 3')
    approveCommand(project, viewId, 'count', true)
    const result = await runCommand(project, viewId, 'count')
    expect(result).toMatchObject({ code: 3, stderr: 'oops\n' })
  })
})

describe('the page’s window.nodus', () => {
  it('shows a page the other views, with their data, and not itself', async () => {
    const a = await createViewFromTool(project, 's1', { title: 'A', html, data: { a: 1 } })
    const b = await createViewFromTool(project, 's1', { title: 'B', html, data: { b: 2 } })
    expect(await callView(project, a.view_id, 'views', {})).toEqual([
      { id: b.view_id, title: 'B', description: 'B', data: { b: 2 } }
    ])
    expect(await callView(project, a.view_id, 'sessions', {})).toEqual([{ id: 's1', title: 'Card' }])
  })

  it('keeps a page’s state, within bounds', async () => {
    const { view_id } = await createViewFromTool(project, 's1', { title: 'A', html })
    await callView(project, view_id, 'setState', { state: { filter: 'open' } })
    expect(getViewForTool(project, 's1', view_id).state).toEqual({ filter: 'open' })
    await expect(callView(project, view_id, 'setState', { state: 'x'.repeat(300_000) })).rejects.toThrow('too big')
  })

  it('opens web links only, and refuses anything else', async () => {
    const { view_id } = await createViewFromTool(project, 's1', { title: 'A', html })
    await expect(callView(project, view_id, 'openUrl', { url: 'file:///etc/passwd' })).rejects.toThrow('Only http')
    await expect(callView(project, view_id, 'readFile', {})).rejects.toThrow('isn’t something a view can do')
  })

  it('asks the brain no more than every 15 seconds', async () => {
    const { view_id } = await createViewFromTool(project, 's1', { title: 'A', html })
    await callView(project, view_id, 'ask', { prompt: 'Fetch fresh data' })
    await expect(callView(project, view_id, 'ask', { prompt: 'Again' })).rejects.toThrow('Asked too soon')
  })
})

describe('serving and removing', () => {
  it('serves a view’s page with the policy, and nothing else', async () => {
    const { view_id } = await createViewFromTool(project, 's1', { title: 'A', html })
    const res = servePage(`nodus-view://${view_id}/?v=1`)
    expect(res.status).toBe(200)
    expect(res.headers.get('content-security-policy')).toContain("connect-src 'none'")
    expect(await res.text()).toContain('<p id="n"></p>')
    expect(servePage('nodus-view://00000000-0000-0000-0000-000000000000/').status).toBe(404)
    expect(servePage(`nodus-view://${view_id}/../state.json`).status).toBe(404)
  })

  it('takes a view off the board with its files and its brain', async () => {
    const { view_id } = await createViewFromTool(project, 's1', { title: 'A', html })
    const board = getBoard(project)
    board.nodes.brain = { sessionId: 'brain', x: 0, y: 0, archived: false, viewOf: view_id, createdAt: 0 }
    ;(board.views![view_id] as BoardView).sessionId = 'brain'
    removeView(project, view_id)
    expect(board.views![view_id]).toBeUndefined()
    expect(board.nodes.brain).toBeUndefined()
    expect(board.hidden).toContain('brain')
    expect(fs.existsSync(path.join(userData, 'views', view_id))).toBe(false)
  })
})
