// Views: small live web pages on the board that Claude builds from what the
// user describes (BoardView in shared/types.ts).
//
// A view's page and what it holds at run time (its data, its saved state, its
// commands' last output, the errors it hit) live in userData/views/<id>/,
// beside state.json and never in the repository. The page is served on the
// nodus-view scheme into a sandboxed frame (viewPage.ts), and everything it
// asks the board for comes through callView, for that view alone.
//
// Fresh data comes from commands the user allowed on the card, which run here
// in the project folder, and from the view's brain: a Claude session of its
// own, hidden from the board like an action run, that builds the page and
// updates it when asked or on a schedule. A brain stops after a few idle
// minutes and is resumed when it's next needed.
import { app, shell } from 'electron'
import { spawn, type ChildProcess } from 'child_process'
import crypto from 'crypto'
import fs from 'fs'
import path from 'path'
import {
  GAP,
  cardSize,
  clampViewSize,
  dockRect,
  findFreeSpot,
  noteSize,
  viewSize,
  type Point,
  type Size
} from '@shared/geometry'
import type {
  BoardNode,
  BoardView,
  CommandResult,
  CreateViewOptions,
  Rect,
  SessionStatus,
  ViewCommand,
  ViewEvent,
  ViewPatch,
  ViewRefresh
} from '@shared/types'
import { hasTranscript, sessionList } from './board'
import { spawnEnv, userShell } from './env'
import { gitStatus } from './git'
import { forgetStatus, getStatus } from './hooks'
import * as ptys from './pty'
import { getBoard, getProject, getState, projectIdForSession, projectIdForView, save, type BoardState } from './store'
import { EMPTY_PAGE, VIEW_CSP, VIEW_GUIDE, buildViewPage } from './viewPage'

const MAX_HTML_BYTES = 2 * 1024 * 1024
const MAX_DATA_CHARS = 1024 * 1024
const MAX_STATE_CHARS = 256 * 1024
const MAX_TITLE_CHARS = 80
const MAX_DESCRIPTION_CHARS = 4000
const MAX_PROMPT_CHARS = 4000
const MAX_COMMANDS = 12
const MAX_COMMAND_CHARS = 2000
const MAX_STDOUT_CHARS = 1024 * 1024
const MAX_STDERR_CHARS = 64 * 1024
/** Commands run at most every minute; the brain is asked at most every ten, as each refresh costs tokens. */
const MIN_COMMAND_EVERY = 1
const MIN_REFRESH_EVERY = 10
const MAX_EVERY = 7 * 24 * 60
const COMMAND_TIMEOUT_MS = 2 * 60 * 1000
/** A page can ask its brain something at most this often. */
const PAGE_ASK_GAP_MS = 15_000
const BRAIN_IDLE_MS = 5 * 60 * 1000
const TICK_MS = 30_000
/** How long a board tool waits for a changed page to load, and then for its scripts to fail. */
const LOAD_WAIT_MS = 3000
const SETTLE_MS = 500
const KEPT_ERRORS = 20
/** How many command lines a view remembers the user's answer for. */
const REMEMBERED_RUNS = 50

/** What a view holds at run time, kept in userData/views/<id>/runtime.json. */
interface Runtime {
  data?: unknown
  state?: unknown
  results: Record<string, CommandResult>
  errors: { message: string; at: number }[]
  /** When its brain was last asked to refresh it. */
  refreshedAt?: number
}

interface ViewHooks {
  /** Sends a view's page an event, through the board. */
  notify(projectId: string, viewId: string, event: ViewEvent, payload: unknown): void
  /** The board's layout changed; the renderer reloads it. */
  boardChanged(projectId: string): void
  /** Resolves once the login-shell environment is in, which commands and git need. */
  envReady: Promise<unknown>
}

let hooks: ViewHooks = { notify() {}, boardChanged() {}, envReady: Promise.resolve() }
let ticker: NodeJS.Timeout | null = null

export function configureViews(h: ViewHooks) {
  hooks = h
  ticker ??= setInterval(tick, TICK_MS)
}

// ---------------------------------------------------------------------------
// What Claude is told about views

const brainPrompt = (view: BoardView) => `You are the brain of a view on the user's Nodus board: a small live web page on a card, beside their Claude Code sessions. The view is "${view.title}" (id ${view.id}). The user asked for:

${view.description}

You build the page and keep it up to date with the mcp__nodus__* board tools. update_view sets its html, data, title, commands and refresh schedule. get_view shows it as it is now, with its commands' last output and any JavaScript errors it hit. Leave out view_id: both act on this view. If you're picking up a view that already has a page, read it with get_view before changing it.

${VIEW_GUIDE}

Nobody watches you while you work, so don't stop to chat. Work out where the data comes from by looking around the repository, the CLIs on the PATH and your MCP tools. If you need the user, for a missing API token or a choice only they can make, ask with AskUserQuestion, which shows on the view that you need them. End each turn with one short line on what you did.`

/** Extra `claude` options for a view's brain, every time it starts or resumes. */
export function brainArgs(projectId: string, node: BoardNode): string[] | undefined {
  const view = node.viewOf ? getBoard(projectId).views![node.viewOf] : undefined
  return view ? ['--append-system-prompt', brainPrompt(view)] : undefined
}

const brainTitle = (view: BoardView) => `View: ${view.title}`

// ---------------------------------------------------------------------------
// Storage

const runtimes = new Map<string, Runtime>()
const runtimeTimers = new Map<string, NodeJS.Timeout>()
const VIEW_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/

function viewDir(viewId: string) {
  // The id ends up in a path.
  if (!VIEW_ID.test(viewId)) throw new Error('Not a view id')
  return path.join(app.getPath('userData'), 'views', viewId)
}

function readHtml(viewId: string): string {
  try {
    return fs.readFileSync(path.join(viewDir(viewId), 'index.html'), 'utf8')
  } catch {
    return ''
  }
}

function writeHtml(viewId: string, html: string) {
  const dir = viewDir(viewId)
  fs.mkdirSync(dir, { recursive: true })
  fs.writeFileSync(path.join(dir, 'index.html'), html)
}

function runtime(viewId: string): Runtime {
  let rt = runtimes.get(viewId)
  if (rt) return rt
  try {
    const raw = JSON.parse(fs.readFileSync(path.join(viewDir(viewId), 'runtime.json'), 'utf8')) as Partial<Runtime>
    rt = {
      data: raw.data,
      state: raw.state,
      results: raw.results && typeof raw.results === 'object' ? raw.results : {},
      errors: Array.isArray(raw.errors) ? raw.errors : [],
      refreshedAt: typeof raw.refreshedAt === 'number' ? raw.refreshedAt : undefined
    }
  } catch {
    rt = { results: {}, errors: [] }
  }
  runtimes.set(viewId, rt)
  return rt
}

function writeRuntime(viewId: string) {
  const rt = runtimes.get(viewId)
  if (!rt) return
  const dir = viewDir(viewId)
  fs.mkdirSync(dir, { recursive: true })
  const file = path.join(dir, 'runtime.json')
  const tmp = `${file}.${process.pid}.tmp`
  fs.writeFileSync(tmp, JSON.stringify(rt))
  fs.renameSync(tmp, file)
}

function saveRuntime(viewId: string) {
  clearTimeout(runtimeTimers.get(viewId))
  runtimeTimers.set(
    viewId,
    setTimeout(() => {
      runtimeTimers.delete(viewId)
      try {
        writeRuntime(viewId)
      } catch (err) {
        console.error('[views] save failed', err)
      }
    }, 500)
  )
}

/** Writes what's pending (on quit). */
export function flushViews() {
  for (const [viewId, timer] of runtimeTimers) {
    clearTimeout(timer)
    try {
      writeRuntime(viewId)
    } catch (err) {
      console.error('[views] flush failed', err)
    }
  }
  runtimeTimers.clear()
  if (ticker) clearInterval(ticker)
  ticker = null
}

// ---------------------------------------------------------------------------
// Checking what Claude and the renderer send

const clip = (s: unknown, max: number) => (typeof s === 'string' ? s.trim().slice(0, max) : '')

const clampEvery = (value: unknown, min: number): number | undefined =>
  typeof value === 'number' && Number.isFinite(value) && value > 0
    ? Math.min(MAX_EVERY, Math.max(min, Math.round(value)))
    : undefined

/**
 * A view's commands from Claude. A command line the user already allowed (or
 * turned down) for this view stays allowed (or turned down), whatever it's
 * called now; a new or changed one waits for them.
 */
export function normalizeCommands(
  raw: unknown,
  remembered: Pick<BoardView, 'allowedRuns' | 'deniedRuns'> = {}
): ViewCommand[] {
  if (!Array.isArray(raw)) throw new Error('commands must be a list of { name, run, every }')
  if (raw.length > MAX_COMMANDS) throw new Error(`A view can have at most ${MAX_COMMANDS} commands`)
  const out: ViewCommand[] = []
  for (const item of raw as Partial<ViewCommand>[]) {
    const name = clip(item?.name, 40)
    const run = clip(item?.run, MAX_COMMAND_CHARS)
    if (!name || !run) throw new Error('Each command needs a name and a run (its command line)')
    if (out.some((c) => c.name === name)) throw new Error(`There are two commands called “${name}”`)
    out.push({
      name,
      run,
      every: clampEvery(item.every, MIN_COMMAND_EVERY),
      approved: remembered.allowedRuns?.includes(run) ? run : undefined,
      denied: remembered.deniedRuns?.includes(run) ? run : undefined
    })
  }
  return out
}

export function normalizeRefresh(raw: unknown): ViewRefresh | undefined {
  if (raw === null || raw === undefined) return undefined
  const r = raw as Partial<ViewRefresh>
  const prompt = clip(r.prompt, MAX_PROMPT_CHARS)
  const every = clampEvery(r.every, MIN_REFRESH_EVERY)
  if (!prompt || !every) throw new Error('refresh needs every (minutes) and a prompt, or null to stop refreshing')
  return { every, prompt }
}

/** JSON-able `value` within `max` characters, or an error naming `what`. */
function checkJson(value: unknown, max: number, what: string) {
  let json: string | undefined
  try {
    json = JSON.stringify(value)
  } catch {
    throw new Error(`${what} must be JSON`)
  }
  if (json !== undefined && json.length > max) throw new Error(`${what} is too big (over ${Math.round(max / 1024)} KB)`)
}

function checkHtml(html: unknown): string {
  if (typeof html !== 'string' || !html.trim()) throw new Error('html must be the page, as one HTML document')
  if (Buffer.byteLength(html) > MAX_HTML_BYTES) throw new Error('The page is too big (over 2 MB)')
  return html
}

const isCommandWaiting = (c: ViewCommand) => c.approved !== c.run && c.denied !== c.run

/** A view, by an id from Claude or the renderer, on this board only. */
function findView(projectId: string, viewId: unknown): BoardView {
  const views = getBoard(projectId).views!
  // Ids come from Claude: "__proto__" must not reach Object.prototype.
  if (typeof viewId !== 'string' || !Object.hasOwn(views, viewId)) {
    throw new Error('There’s no view with that id on this board')
  }
  return views[viewId]
}

// ---------------------------------------------------------------------------
// Serving pages

/** Answers the frame's request for nodus-view://<id>/. */
export function servePage(url: string): Response {
  let viewId = ''
  let pathname = ''
  try {
    const u = new URL(url)
    viewId = u.hostname
    pathname = u.pathname
  } catch {
    // Not found, below.
  }
  const projectId = projectIdForView(viewId)
  if (!projectId || pathname !== '/') return new Response('Not found', { status: 404 })
  const view = getBoard(projectId).views![viewId]
  const rt = runtime(viewId)
  const page = buildViewPage(readHtml(viewId) || EMPTY_PAGE, {
    viewId,
    title: view.title,
    data: rt.data,
    state: rt.state,
    results: rt.results
  })
  return new Response(page, {
    headers: {
      'Content-Type': 'text/html; charset=utf-8',
      'Content-Security-Policy': VIEW_CSP,
      'Cache-Control': 'no-store',
      'X-Content-Type-Options': 'nosniff'
    }
  })
}

// ---------------------------------------------------------------------------
// Placing views

function obstacles(board: BoardState, except?: string): Rect[] {
  const cards = Object.values(board.nodes).filter((n) => !n.archived && !n.runOf && !n.viewOf)
  return [
    ...cards.map((n) => ({ x: n.x, y: n.y, ...cardSize(n) })),
    ...Object.values(board.notes!)
      .filter((n) => !n.sessionId)
      .map((n) => ({ x: n.x, y: n.y, ...noteSize(n) })),
    ...Object.values(board.views!)
      .filter((v) => v.id !== except)
      .map((v) => ({ x: v.x, y: v.y, ...viewSize(v) }))
  ]
}

/** Beside the card of the session putting it up, or the rightmost card, in the nearest free spot. */
function placeView(board: BoardState, from: string, size: Size): Point {
  const cards = Object.values(board.nodes).filter((n) => !n.archived && !n.runOf && !n.viewOf)
  const dock = dockRect(board.archive)
  const card =
    cards.find((n) => n.sessionId === from) ??
    cards.reduce<BoardNode | undefined>((a, b) => (a && a.x >= b.x ? a : b), undefined)
  const preferred = card ? { x: card.x + cardSize(card).w + GAP, y: card.y } : { x: dock.x + dock.w + 80, y: dock.y }
  return findFreeSpot(preferred, obstacles(board), dock, size)
}

// ---------------------------------------------------------------------------
// Views from the renderer

/** Puts up a view for what the user described, and starts its brain on building it. */
export async function createView(projectId: string, opts: CreateViewOptions): Promise<BoardView> {
  const description = clip(opts?.description, MAX_DESCRIPTION_CHARS)
  if (!description) throw new Error('Describe what the view should show')
  if (!Number.isFinite(opts.x) || !Number.isFinite(opts.y)) throw new Error('A view needs a place on the board')
  const board = getBoard(projectId)
  const now = Date.now()
  const view: BoardView = {
    id: crypto.randomUUID(),
    title: 'New view',
    description,
    x: Math.round(opts.x),
    y: Math.round(opts.y),
    version: 0,
    createdAt: now,
    updatedAt: now
  }
  board.views![view.id] = view
  try {
    await startBrain(projectId, view, `Build this view:\n\n${description}`)
  } catch (err) {
    delete board.views![view.id]
    throw err
  }
  save()
  return view
}

export function saveView(projectId: string, patch: ViewPatch) {
  const view = findView(projectId, patch?.id)
  if (Number.isFinite(patch.x)) view.x = Math.round(patch.x!)
  if (Number.isFinite(patch.y)) view.y = Math.round(patch.y!)
  const size = clampViewSize({ w: patch.w, h: patch.h })
  if (size.w) view.w = size.w
  if (size.h) view.h = size.h
  const title = clip(patch.title, MAX_TITLE_CHARS)
  if (title) setTitle(projectId, view, title)
  save()
}

function setTitle(projectId: string, view: BoardView, title: string) {
  view.title = title
  const brain = view.sessionId ? getBoard(projectId).nodes[view.sessionId] : undefined
  if (brain) brain.title = brainTitle(view)
}

/** Takes a view off the board, with its page, its data and its brain. */
export function removeView(projectId: string, viewId: string) {
  const board = getBoard(projectId)
  const view = findView(projectId, viewId)
  delete board.views![viewId]
  const brain = view.sessionId ? board.nodes[view.sessionId] : undefined
  if (brain) {
    // Its transcript stays on disk, hidden like a removed card's.
    delete board.nodes[brain.sessionId]
    if (!board.hidden!.includes(brain.sessionId)) board.hidden!.push(brain.sessionId)
    forgetBrain(brain.sessionId)
    void ptys.kill(brain.sessionId).then(() => forgetStatus(brain.sessionId))
  }
  deleteFiles(viewId)
  save()
}

/** Forgets the files of a project's views when the project goes. Their brains stop with its sessions. */
export function removeProjectViews(board: BoardState | undefined) {
  for (const viewId of Object.keys(board?.views ?? {})) deleteFiles(viewId)
}

function deleteFiles(viewId: string) {
  clearTimeout(runtimeTimers.get(viewId))
  runtimeTimers.delete(viewId)
  runtimes.delete(viewId)
  try {
    fs.rmSync(viewDir(viewId), { recursive: true, force: true })
  } catch (err) {
    console.error('[views] remove failed', err)
  }
}

/** The user allowing (or turning down) one of a view's command lines. One they allow runs straight away. */
export function approveCommand(projectId: string, viewId: string, name: string, allow: boolean) {
  const view = findView(projectId, viewId)
  const cmd = view.commands?.find((c) => c.name === name)
  if (!cmd) throw new Error('That command is gone')
  const without = (runs: string[] | undefined) => (runs ?? []).filter((r) => r !== cmd.run)
  if (allow) {
    cmd.approved = cmd.run
    delete cmd.denied
    view.allowedRuns = [...without(view.allowedRuns), cmd.run].slice(-REMEMBERED_RUNS)
    view.deniedRuns = without(view.deniedRuns)
  } else {
    cmd.denied = cmd.run
    delete cmd.approved
    view.deniedRuns = [...without(view.deniedRuns), cmd.run].slice(-REMEMBERED_RUNS)
    view.allowedRuns = without(view.allowedRuns)
  }
  save()
  hooks.boardChanged(projectId)
  if (allow) runCommand(projectId, viewId, name).catch(() => {})
}

/** Runs a view's allowed commands now, and asks its brain to refresh it if it has a refresh prompt. */
export async function refreshView(projectId: string, viewId: string): Promise<CommandResult[]> {
  const view = findView(projectId, viewId)
  if (view.refresh) {
    runtime(viewId).refreshedAt = Date.now()
    saveRuntime(viewId)
    await askBrain(projectId, viewId, view.refresh.prompt, 'user')
  }
  const allowed = (view.commands ?? []).filter((c) => c.approved === c.run)
  return Promise.all(allowed.map((c) => runCommand(projectId, viewId, c.name)))
}

/** What a view's page reported: an error, or that the page at `loaded` (its version) is up. */
export function reportView(projectId: string, viewId: string, report: { error?: unknown; loaded?: unknown }) {
  const view = findView(projectId, viewId)
  if (typeof report?.error === 'string' && report.error) {
    const rt = runtime(viewId)
    rt.errors = [...rt.errors, { message: report.error.slice(0, 2000), at: Date.now() }].slice(-KEPT_ERRORS)
    saveRuntime(viewId)
  }
  if (typeof report?.loaded === 'number' && report.loaded >= view.version) {
    for (const resolve of loadWaiters.get(viewId) ?? []) resolve(true)
    loadWaiters.delete(viewId)
  }
}

// ---------------------------------------------------------------------------
// The page's window.nodus

const lastPageAsk = new Map<string, number>()

/** A request from a view's page, answered for that view and its board only. */
export async function callView(projectId: string, viewId: string, method: string, args: unknown): Promise<unknown> {
  const view = findView(projectId, viewId)
  const a = (args && typeof args === 'object' ? args : {}) as Record<string, unknown>
  const board = getBoard(projectId)
  switch (method) {
    case 'sessions':
      return sessionList(getProject(projectId), null, false)
    case 'notes':
      return Object.values(board.notes!)
        .sort((x, y) => x.createdAt - y.createdAt)
        .map((n) => ({ id: n.id, text: n.text, color: n.color }))
    case 'views':
      return Object.values(board.views!)
        .filter((v) => v.id !== viewId)
        .map(viewSummary)
    case 'view':
      return viewSummary(findView(projectId, a.id))
    case 'git':
      await hooks.envReady
      return gitStatus(getProject(projectId).repoPath)
    case 'setState': {
      checkJson(a.state, MAX_STATE_CHARS, 'The state')
      const rt = runtime(viewId)
      rt.state = a.state
      saveRuntime(viewId)
      hooks.notify(projectId, viewId, 'state', a.state)
      return null
    }
    case 'run':
      return runCommand(projectId, viewId, String(a.name ?? ''))
    case 'ask': {
      const last = lastPageAsk.get(viewId) ?? 0
      if (Date.now() - last < PAGE_ASK_GAP_MS) throw new Error('Asked too soon: a view can ask its brain once every 15 seconds')
      lastPageAsk.set(viewId, Date.now())
      await askBrain(projectId, view.id, String(a.prompt ?? ''), 'page')
      return null
    }
    case 'openUrl': {
      const url = typeof a.url === 'string' ? a.url : ''
      if (!/^https?:\/\//i.test(url)) throw new Error('Only http and https links open')
      await shell.openExternal(url)
      return null
    }
    default:
      throw new Error(`nodus.${method} isn’t something a view can do`)
  }
}

const viewSummary = (v: BoardView) => ({ id: v.id, title: v.title, description: v.description, data: runtime(v.id).data })

// ---------------------------------------------------------------------------
// Commands

const runningCommands = new Map<string, Promise<CommandResult>>()

/** Runs one of a view's commands, if the user allowed its command line. A run already going is shared. */
export function runCommand(projectId: string, viewId: string, name: string): Promise<CommandResult> {
  const view = findView(projectId, viewId)
  const cmd = view.commands?.find((c) => c.name === name)
  if (!cmd) return Promise.reject(new Error(`This view has no command called “${name}”`))
  if (cmd.approved !== cmd.run) {
    return Promise.reject(
      new Error(
        cmd.denied === cmd.run
          ? `The user didn’t allow “${name}” to run`
          : `“${name}” is waiting for the user to allow it on the view`
      )
    )
  }
  const key = `${viewId}\0${name}`
  const going = runningCommands.get(key)
  if (going) return going
  const run = execute(getProject(projectId).repoPath, cmd)
    .then((result) => {
      // The view may have gone while it ran.
      if (getBoard(projectId).views![viewId]) {
        runtime(viewId).results[name] = result
        saveRuntime(viewId)
        hooks.notify(projectId, viewId, 'result', result)
      }
      return result
    })
    .finally(() => runningCommands.delete(key))
  runningCommands.set(key, run)
  return run
}

/** Runs a command line in the user's shell, with their environment, and collects what it prints. */
async function execute(cwd: string, cmd: ViewCommand): Promise<CommandResult> {
  await hooks.envReady
  const started = Date.now()
  return new Promise((resolve) => {
    let stdout = ''
    let stderr = ''
    let timedOut = false
    const done = (code: number | null, error?: string) =>
      resolve({
        name: cmd.name,
        code,
        stdout: stdout.slice(0, MAX_STDOUT_CHARS),
        stderr: stderr.slice(0, MAX_STDERR_CHARS),
        at: Date.now(),
        durationMs: Date.now() - started,
        error
      })
    let child: ChildProcess
    try {
      // Its own process group, so a timeout stops everything it started.
      child = spawn(userShell(), ['-c', cmd.run], {
        cwd,
        env: spawnEnv({ NO_COLOR: '1' }),
        stdio: ['ignore', 'pipe', 'pipe'],
        detached: true
      })
    } catch (err) {
      done(null, err instanceof Error ? err.message : String(err))
      return
    }
    const timer = setTimeout(() => {
      timedOut = true
      try {
        process.kill(-child.pid!, 'SIGKILL')
      } catch {
        child.kill('SIGKILL')
      }
    }, COMMAND_TIMEOUT_MS)
    child.stdout!.on('data', (d: Buffer) => {
      if (stdout.length < MAX_STDOUT_CHARS) stdout += d.toString('utf8')
    })
    child.stderr!.on('data', (d: Buffer) => {
      if (stderr.length < MAX_STDERR_CHARS) stderr += d.toString('utf8')
    })
    child.on('error', (err) => {
      clearTimeout(timer)
      done(null, err.message)
    })
    child.on('close', (code) => {
      clearTimeout(timer)
      done(timedOut ? null : code, timedOut ? 'It took longer than 2 minutes, so it was stopped' : undefined)
    })
  })
}

// ---------------------------------------------------------------------------
// Brains

/** What a brain was asked while it was busy, for when its turn ends. */
const pendingAsks = new Map<string, string>()
const idleTimers = new Map<string, NodeJS.Timeout>()
const lastInput = new Map<string, number>()

/** Starts a fresh brain for a view, on `prompt` if there is one. */
async function startBrain(projectId: string, view: BoardView, prompt?: string): Promise<string> {
  const project = getProject(projectId)
  const board = getBoard(projectId)
  const node: BoardNode = {
    sessionId: crypto.randomUUID(),
    x: view.x,
    y: view.y,
    archived: false,
    title: brainTitle(view),
    createdByApp: true,
    viewOf: view.id,
    autoMode: true,
    createdAt: Date.now()
  }
  const previous = view.sessionId
  board.nodes[node.sessionId] = node
  view.sessionId = node.sessionId
  try {
    await ptys.spawnSession({
      sessionId: node.sessionId,
      projectId,
      cwd: project.repoPath,
      resume: false,
      initialPrompt: prompt,
      permissionMode: 'auto',
      extraArgs: brainArgs(projectId, node)
    })
  } catch (err) {
    delete board.nodes[node.sessionId]
    view.sessionId = previous
    throw err
  }
  save()
  hooks.boardChanged(projectId)
  return node.sessionId
}

/** The view's brain, started idle if it has none. */
export async function ensureBrain(projectId: string, viewId: string): Promise<string> {
  const view = findView(projectId, viewId)
  const node = view.sessionId ? getBoard(projectId).nodes[view.sessionId] : undefined
  return node ? node.sessionId : startBrain(projectId, view)
}

/**
 * Passes `prompt` to a view's brain: typed in if it's waiting at its prompt,
 * queued for when its turn ends if it's busy, or with the brain resumed (or
 * a first one started) if it isn't running. A scheduled refresh skips a busy
 * brain rather than queue behind it.
 */
export async function askBrain(
  projectId: string,
  viewId: string,
  prompt: string,
  origin: 'user' | 'page' | 'schedule'
): Promise<void> {
  const view = findView(projectId, viewId)
  const text = clip(prompt, MAX_PROMPT_CHARS)
  if (!text) throw new Error('Say what the view’s brain should do')
  const board = getBoard(projectId)
  const node = view.sessionId ? board.nodes[view.sessionId] : undefined
  if (!node) {
    await startBrain(projectId, view, text)
    return
  }
  const id = node.sessionId
  if (ptys.isLive(id)) {
    if (getStatus(id) === 'done') typeInto(id, text)
    else if (origin !== 'schedule') {
      const queued = pendingAsks.get(id)
      pendingAsks.set(id, queued ? `${queued}\n\n${text}`.slice(-MAX_PROMPT_CHARS) : text)
    }
    return
  }
  const project = getProject(projectId)
  await ptys.spawnSession({
    sessionId: id,
    projectId,
    cwd: project.repoPath,
    resume: hasTranscript(project, id),
    initialPrompt: text,
    permissionMode: 'auto',
    extraArgs: brainArgs(projectId, node)
  })
}

/** Types a prompt at Claude's prompt as a paste, so lines don't send it early, then sends it. */
function typeInto(sessionId: string, text: string) {
  clearTimeout(idleTimers.get(sessionId))
  ptys.write(sessionId, `\x1b[200~${text}\x1b[201~`)
  setTimeout(() => ptys.write(sessionId, '\r'), 200)
}

/**
 * A brain's status changed. When its turn ends it takes what was asked
 * meanwhile, or stops once it has been idle for a while: it's resumed when
 * it's next needed, and nobody is looking at it in between.
 */
export function brainStatus(sessionId: string, status: SessionStatus, live: boolean) {
  const projectId = projectIdForSession(sessionId)
  if (!projectId || !getBoard(projectId).nodes[sessionId]?.viewOf) return
  clearTimeout(idleTimers.get(sessionId))
  idleTimers.delete(sessionId)
  if (status !== 'done' || !live) return
  const queued = pendingAsks.get(sessionId)
  if (queued) {
    pendingAsks.delete(sessionId)
    // Claude's prompt takes a moment to come back after the turn ends.
    setTimeout(() => typeInto(sessionId, queued), 500)
    return
  }
  idleTimers.set(
    sessionId,
    setTimeout(() => {
      idleTimers.delete(sessionId)
      const quiet = Date.now() - (lastInput.get(sessionId) ?? 0) >= BRAIN_IDLE_MS
      if (getStatus(sessionId) === 'done' && quiet) void ptys.kill(sessionId)
      else if (getStatus(sessionId) === 'done') brainStatus(sessionId, 'done', ptys.isLive(sessionId))
    }, BRAIN_IDLE_MS)
  )
}

/** The user typed into a session; an idle brain they're talking to isn't idle. */
export function brainInput(sessionId: string) {
  if (idleTimers.has(sessionId)) lastInput.set(sessionId, Date.now())
}

function forgetBrain(sessionId: string) {
  clearTimeout(idleTimers.get(sessionId))
  idleTimers.delete(sessionId)
  pendingAsks.delete(sessionId)
  lastInput.delete(sessionId)
}

// ---------------------------------------------------------------------------
// Schedules

/** Runs the commands and refreshes that are due, while the app is open. */
function tick() {
  const now = Date.now()
  const state = getState()
  for (const project of state.projects) {
    if (project.archived) continue
    for (const view of Object.values(state.boards[project.id]?.views ?? {})) {
      const rt = runtime(view.id)
      for (const cmd of view.commands ?? []) {
        if (!cmd.every || cmd.approved !== cmd.run) continue
        if (now - (rt.results[cmd.name]?.at ?? 0) < cmd.every * 60_000) continue
        runCommand(project.id, view.id, cmd.name).catch((err) => console.warn('[views] command failed', err))
      }
      if (view.refresh && now - (rt.refreshedAt ?? view.createdAt) >= view.refresh.every * 60_000) {
        rt.refreshedAt = now
        saveRuntime(view.id)
        askBrain(project.id, view.id, view.refresh.prompt, 'schedule').catch((err) =>
          console.warn('[views] refresh failed', err)
        )
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Board tools

const loadWaiters = new Map<string, ((loaded: boolean) => void)[]>()

/** Waits for the board to load a changed page, then for its scripts to settle, and returns what went wrong. */
async function checkPage(viewId: string, since: number) {
  const loaded = await new Promise<boolean>((resolve) => {
    const timer = setTimeout(() => {
      loadWaiters.set(viewId, (loadWaiters.get(viewId) ?? []).filter((r) => r !== settle))
      resolve(false)
    }, LOAD_WAIT_MS)
    const settle = (ok: boolean) => {
      clearTimeout(timer)
      resolve(ok)
    }
    loadWaiters.set(viewId, [...(loadWaiters.get(viewId) ?? []), settle])
  })
  if (!loaded) {
    return { checked: 'The board isn’t showing this view right now, so the page couldn’t be checked for errors.' }
  }
  await new Promise((r) => setTimeout(r, SETTLE_MS))
  const errors = runtime(viewId).errors.filter((e) => e.at >= since).map((e) => e.message)
  return errors.length ? { errors } : { errors: [] }
}

/** The calling session's own view, when it's a view's brain. */
function ownView(projectId: string, self: string): BoardView {
  const viewOf = getBoard(projectId).nodes[self]?.viewOf
  if (!viewOf) throw new Error('view_id is required: this session isn’t a view’s brain')
  return findView(projectId, viewOf)
}

const waitingNames = (view: BoardView) => (view.commands ?? []).filter(isCommandWaiting).map((c) => c.name)

export interface ViewToolArgs {
  view_id?: unknown
  title?: unknown
  description?: unknown
  html?: unknown
  data?: unknown
  commands?: unknown
  refresh?: unknown
  width?: unknown
  height?: unknown
}

/** create_view: a session puts up a view of its own making, beside its card. Its brain starts when first needed. */
export async function createViewFromTool(projectId: string, self: string, args: ViewToolArgs) {
  const title = clip(args.title, MAX_TITLE_CHARS)
  if (!title) throw new Error('A view needs a title')
  const html = checkHtml(args.html)
  if (args.data !== undefined) checkJson(args.data, MAX_DATA_CHARS, 'The data')
  const commands = args.commands === undefined ? undefined : normalizeCommands(args.commands)
  const refresh = normalizeRefresh(args.refresh)
  const board = getBoard(projectId)
  const size = clampViewSize({ w: Number(args.width), h: Number(args.height) })
  const now = Date.now()
  const view: BoardView = {
    id: crypto.randomUUID(),
    title,
    description: clip(args.description, MAX_DESCRIPTION_CHARS) || title,
    ...placeView(board, self, viewSize(size)),
    w: size.w,
    h: size.h,
    from: self,
    commands: commands?.length ? commands : undefined,
    refresh,
    version: 1,
    createdAt: now,
    updatedAt: now
  }
  writeHtml(view.id, html)
  const rt = runtime(view.id)
  rt.data = args.data
  if (refresh) rt.refreshedAt = now
  saveRuntime(view.id)
  board.views![view.id] = view
  save()
  hooks.boardChanged(projectId)
  const page = await checkPage(view.id, now)
  return { view_id: view.id, waiting_for_approval: waitingNames(view), ...page }
}

/** update_view: changes a view's page, data, title, commands or refresh schedule. */
export async function updateViewFromTool(projectId: string, self: string, args: ViewToolArgs) {
  const view = args.view_id === undefined ? ownView(projectId, self) : findView(projectId, args.view_id)
  const html = args.html === undefined ? undefined : checkHtml(args.html)
  if (args.data !== undefined) checkJson(args.data, MAX_DATA_CHARS, 'The data')
  const commands = args.commands === undefined ? undefined : normalizeCommands(args.commands, view)
  const refresh = args.refresh === undefined ? view.refresh : normalizeRefresh(args.refresh)
  const title = clip(args.title, MAX_TITLE_CHARS)
  const now = Date.now()
  const rt = runtime(view.id)

  if (title) setTitle(projectId, view, title)
  if (args.description !== undefined) view.description = clip(args.description, MAX_DESCRIPTION_CHARS) || view.description
  if (commands) view.commands = commands.length ? commands : undefined
  if (args.refresh !== undefined) {
    view.refresh = refresh
    rt.refreshedAt = now
  }
  const size = clampViewSize({ w: Number(args.width), h: Number(args.height) })
  if (size.w) view.w = size.w
  if (size.h) view.h = size.h
  if (html !== undefined) {
    writeHtml(view.id, html)
    view.version++
  }
  if (args.data !== undefined) {
    rt.data = args.data
    // A page about to reload reads it as it starts.
    if (html === undefined) hooks.notify(projectId, view.id, 'data', args.data)
  }
  view.updatedAt = now
  saveRuntime(view.id)
  save()
  hooks.boardChanged(projectId)
  const page = html !== undefined ? await checkPage(view.id, now) : {}
  return { view_id: view.id, version: view.version, waiting_for_approval: waitingNames(view), ...page }
}

/** get_view: a view as it is now, for Claude to work on. */
export function getViewForTool(projectId: string, self: string, viewId: unknown) {
  const view = viewId === undefined ? ownView(projectId, self) : findView(projectId, viewId)
  const rt = runtime(view.id)
  const brief = (s: string) => (s.length > 4000 ? `${s.slice(0, 4000)}… (${s.length - 4000} more characters)` : s)
  return {
    id: view.id,
    title: view.title,
    description: view.description,
    html: readHtml(view.id),
    data: rt.data,
    state: rt.state,
    commands: (view.commands ?? []).map((c) => ({
      name: c.name,
      run: c.run,
      every: c.every,
      allowed: c.approved === c.run ? 'yes' : c.denied === c.run ? 'no, the user turned it down' : 'waiting for the user',
      last: rt.results[c.name] && {
        ...rt.results[c.name],
        stdout: brief(rt.results[c.name].stdout),
        stderr: brief(rt.results[c.name].stderr)
      }
    })),
    refresh: view.refresh,
    errors: rt.errors.slice(-10).map((e) => ({ message: e.message, at: new Date(e.at).toISOString() }))
  }
}

/** list_views: the board's views. */
export function listViewsForTool(projectId: string, self: string) {
  return Object.values(getBoard(projectId).views!)
    .sort((a, b) => a.createdAt - b.createdAt)
    .map((v) => ({
      id: v.id,
      title: v.title,
      description: v.description,
      yours: v.sessionId === self || v.from === self || undefined
    }))
}
