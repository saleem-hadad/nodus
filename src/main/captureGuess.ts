// Claude's guess at what a floating capture is and where it goes.
//
// A quick headless claude call, like naming a section (naming.ts): Haiku with
// thinking, tools and MCP servers off, outside any repository. It reads the
// projects, their recent sessions, the app in front and what was typed, and
// answers in a JSON schema. The panel asks again each time the typing pauses,
// and a new question stops the one before it.
import { spawn, type ChildProcess } from 'child_process'
import os from 'os'
import { CAPTURE_KINDS, isCaptureKind } from '@shared/capture'
import type { CaptureAttachment, CaptureGuess, CaptureKind, CaptureTargets } from '@shared/types'
import { requireClaude, spawnEnv } from './env'

const TIMEOUT_MS = 30_000
const MAX_TEXT_CHARS = 4000
/** Recent sessions listed for each project. */
const SESSIONS_PER_PROJECT = 8
const MAX_REASON_CHARS = 120

const SYSTEM_PROMPT = `You route quick captures on Nodus, a board of Claude Code sessions for each of the user's projects. In another app, the user opened a small panel and typed something. Decide what it becomes and which project it goes to:
- session: a new Claude Code session in the project's repository that does what the text asks. The default for work to be done.
- into: the text is sent to a session that already exists, as the user's next message to it. Only when the text clearly carries on that session's work: it follows up on it ("also…", "and add tests for it"), replies to it, or names it. Never a session that is waiting for the user.
- note: a sticky note on the project's board, and nothing runs. For ideas, reminders and things to do later or to ask someone, not work to start now ("remember to…", "idea:", "todo:", "ask finance about…").
- view: a live page on the board that Claude builds and keeps up to date. Only when the text asks for something to watch or show: a dashboard, a table, a chart, a list or the status of something.

Take the project the text names (a project, a repository, a file or one of its sessions), else the one the browser tab or the app in front points to, else the default project.

Answer with kind, project (an id from the list), session (an id from the list, only for into; otherwise ""), and reason: one short sentence for the user, under 80 characters, saying why. For example: "It follows on from “Fix checkout total rounding”." or "It's an idea to keep, not work to start now."`

const SCHEMA = {
  type: 'object',
  properties: {
    kind: { type: 'string', enum: CAPTURE_KINDS },
    project: { type: 'string' },
    session: { type: 'string' },
    reason: { type: 'string' }
  },
  required: ['kind', 'project', 'session', 'reason'],
  additionalProperties: false
}

/** What the guess goes on besides the text. */
export interface GuessInput {
  text: string
  attachments: CaptureAttachment[]
  /** The app in front when the panel opened. */
  app?: string
  targets: CaptureTargets
}

const ago = (ms: number) => {
  const min = Math.round(ms / 60_000)
  if (min < 1) return 'just now'
  if (min < 60) return `${min} min ago`
  const h = Math.round(min / 60)
  return h < 48 ? `${h} h ago` : `${Math.round(h / 24)} days ago`
}

const flat = (s: string, max: number) => {
  const one = s.replace(/\s+/g, ' ').trim()
  return one.length > max ? `${one.slice(0, max - 1)}…` : one
}

const statusWord = (s: { status: string; live: boolean }) =>
  s.status === 'waiting' ? 'waiting for the user' : s.status === 'working' ? 'working' : s.live ? 'idle' : 'stopped'

export function buildGuessPrompt({ text, attachments, app, targets }: GuessInput, now = Date.now()): string {
  const fallback = targets.projects.find((p) => p.id === targets.defaultProjectId)
  const projects = targets.projects.map((p) => {
    const sessions = targets.sessions
      .filter((s) => s.projectId === p.id)
      .slice(0, SESSIONS_PER_PROJECT)
      .map(
        (s) => `  - session ${s.sessionId}: “${flat(s.title, 80)}”, ${statusWord(s)}, active ${ago(now - s.updatedAt)}`
      )
    return [`- project ${p.id}: ${p.name}`, ...(sessions.length ? sessions : ['  (no sessions)'])].join('\n')
  })

  const screen: string[] = []
  if (app) screen.push(`The app in front: ${app}`)
  for (const a of attachments) {
    if (a.kind === 'tab') screen.push(`Its browser tab: ${flat(a.title, 120)} (${a.url})`)
    else if (a.kind === 'image')
      screen.push(a.copied ? 'An image the user copied is attached.' : 'A screenshot is attached.')
    else if (a.kind === 'file') screen.push(`A file is attached: ${a.name}`)
    else if (a.kind === 'text') screen.push(`Copied text is attached: ${flat(a.text, 200)}`)
  }

  return [
    `Projects${fallback ? ` (default: ${fallback.name}, ${fallback.id})` : ''}:`,
    projects.join('\n'),
    screen.length ? `\n${screen.join('\n')}` : '',
    `\nThe capture:\n"""\n${text.trim().slice(0, MAX_TEXT_CHARS)}\n"""`
  ].join('\n')
}

/**
 * Claude's answer, checked against what exists: a project it made up falls
 * back to the default, and "into" a session that isn't there, or is waiting
 * on the user, becomes a new session.
 */
export function readGuess(raw: unknown, targets: CaptureTargets): CaptureGuess | null {
  if (!raw || typeof raw !== 'object') return null
  const r = raw as Record<string, unknown>
  const projectIds = new Set(targets.projects.map((p) => p.id))
  const projectId = typeof r.project === 'string' && projectIds.has(r.project) ? r.project : targets.defaultProjectId
  if (!projectId) return null
  let kind: CaptureKind = isCaptureKind(r.kind) ? r.kind : 'session'
  let sessionId: string | undefined
  if (kind === 'into') {
    const target = targets.sessions.find((s) => s.sessionId === r.session)
    if (target && target.status !== 'waiting') sessionId = target.sessionId
    else kind = 'session'
  }
  const reason = typeof r.reason === 'string' ? flat(r.reason, MAX_REASON_CHARS) : ''
  return {
    kind,
    // A session belongs to its own project, whatever project was named.
    projectId: sessionId ? targets.sessions.find((s) => s.sessionId === sessionId)!.projectId : projectId,
    sessionId,
    reason
  }
}

let running: { child: ChildProcess; settle: (guess: CaptureGuess | null) => void } | null = null

/** Stops a guess still in progress; it resolves null. */
export function cancelGuess() {
  if (!running) return
  const { child, settle } = running
  running = null
  child.kill('SIGKILL')
  settle(null)
}

/** Asks Claude, resolving null if it can't say or a newer guess replaced this one. */
export function guessCapture(input: GuessInput): Promise<CaptureGuess | null> {
  cancelGuess()
  if (!input.text.trim() || !input.targets.projects.length) return Promise.resolve(null)
  const claude = requireClaude()
  const args = [
    '-p',
    '--model',
    'haiku',
    '--settings',
    JSON.stringify({ alwaysThinkingEnabled: false }),
    '--tools',
    '',
    '--strict-mcp-config',
    '--no-session-persistence',
    '--json-schema',
    JSON.stringify(SCHEMA),
    '--output-format',
    'json',
    '--system-prompt',
    SYSTEM_PROMPT,
    buildGuessPrompt(input)
  ]
  return new Promise((resolve) => {
    // stdin is ignored, otherwise claude -p waits for piped input.
    const child = spawn(claude, args, {
      cwd: os.tmpdir(),
      env: spawnEnv(),
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true
    })
    let done = false
    const settle = (guess: CaptureGuess | null) => {
      if (done) return
      done = true
      clearTimeout(timer)
      if (running?.child === child) running = null
      resolve(guess)
    }
    running = { child, settle }
    let stdout = ''
    child.stdout!.on('data', (d: Buffer) => (stdout += d.toString('utf8')))
    child.stderr!.on('data', () => {})
    const timer = setTimeout(() => {
      child.kill('SIGKILL')
      settle(null)
    }, TIMEOUT_MS)
    child.on('error', (err) => {
      console.error('[capture] guess failed', err)
      settle(null)
    })
    child.on('close', () => {
      try {
        const parsed = JSON.parse(stdout.trim()) as { is_error?: boolean; structured_output?: unknown }
        settle(parsed.is_error ? null : readGuess(parsed.structured_output, input.targets))
      } catch {
        settle(null)
      }
    })
  })
}
