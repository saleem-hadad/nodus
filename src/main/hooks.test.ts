import { spawn } from 'child_process'
import fs from 'fs'
import http from 'http'
import type { AddressInfo } from 'net'
import os from 'os'
import path from 'path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SessionUsage } from '@shared/types'
import { gitBash } from './env'
import {
  configureStatus,
  getStatus,
  hookEnv,
  hookSettings,
  markProcess,
  noteUserInput,
  receiveHook,
  setHookEndpoint
} from './hooks'
import { hostPaths, writeHookConfig } from './hostProtocol'

// On Windows, as if Git Bash were there, so hooks are written for a POSIX shell unless a test says otherwise.
vi.mock('./env', () => ({ gitBash: vi.fn(() => (process.platform === 'win32' ? 'bash.exe' : null)) }))

// The status machine keeps state per session id, so every test starts a new one.
let next = 0
const newSession = () => {
  const id = `session-${++next}`
  live.add(id)
  markProcess(id, true)
  return id
}

const live = new Set<string>()
let changes: [string, string, boolean][]
let agentChanges: [string, string, string][]
let usages: [string, SessionUsage][]
const hasPendingWork = vi.fn((_file: string) => false)

beforeEach(() => {
  changes = []
  agentChanges = []
  usages = []
  hasPendingWork.mockReset().mockReturnValue(false)
  configureStatus({
    onChange: (id, status, isLive) => changes.push([id, status, isLive]),
    onAgentChange: (id, agentId, status) => agentChanges.push([id, agentId, status]),
    onUsage: (id, usage) => usages.push([id, usage]),
    isLive: (id) => live.has(id),
    hasPendingWork
  })
})

afterEach(() => {
  vi.useRealTimers()
})

describe('session status', () => {
  it('is working through a turn and done when it stops', () => {
    const id = newSession()
    expect(getStatus(id)).toBe('done')
    receiveHook('UserPromptSubmit', id, {})
    expect(getStatus(id)).toBe('working')
    receiveHook('PreToolUse', id, { tool_name: 'Bash' })
    receiveHook('PostToolUse', id, {})
    expect(getStatus(id)).toBe('working')
    receiveHook('Stop', id, {})
    expect(getStatus(id)).toBe('done')
    // Only changes are reported.
    expect(changes.filter(([s]) => s === id).map(([, status]) => status)).toEqual(['done', 'working', 'done'])
  })

  it('waits for the user on a permission prompt or a question', () => {
    const id = newSession()
    receiveHook('PermissionRequest', id, {})
    expect(getStatus(id)).toBe('waiting')
    receiveHook('PostToolUse', id, {})
    receiveHook('PreToolUse', id, { tool_name: 'AskUserQuestion' })
    expect(getStatus(id)).toBe('waiting')
    receiveHook('Notification', id, { notification_type: 'permission_prompt' })
    expect(getStatus(id)).toBe('waiting')
  })

  it('ignores notifications that are not about permission', () => {
    const id = newSession()
    receiveHook('Notification', id, { notification_type: 'idle_prompt', message: 'Claude is waiting' })
    expect(getStatus(id)).toBe('done')
  })

  it('goes back to working when the user answers a prompt', () => {
    const id = newSession()
    receiveHook('PermissionRequest', id, {})
    noteUserInput(id, '1')
    expect(getStatus(id)).toBe('working')
  })

  it('ends the turn on an interrupt that no hook follows', () => {
    vi.useFakeTimers()
    const id = newSession()
    receiveHook('UserPromptSubmit', id, {})
    noteUserInput(id, '\x1b')
    expect(getStatus(id)).toBe('working')
    vi.advanceTimersByTime(1200)
    expect(getStatus(id)).toBe('done')
  })

  it('keeps working when a hook follows the interrupt', () => {
    vi.useFakeTimers()
    const id = newSession()
    receiveHook('UserPromptSubmit', id, {})
    noteUserInput(id, '\x03')
    vi.advanceTimersByTime(100)
    receiveHook('PostToolUse', id, {})
    vi.advanceTimersByTime(1200)
    expect(getStatus(id)).toBe('working')
  })

  it('reads as done once the process is gone', () => {
    const id = newSession()
    receiveHook('UserPromptSubmit', id, {})
    live.delete(id)
    markProcess(id, false)
    expect(getStatus(id)).toBe('done')
    expect(changes.at(-1)).toEqual([id, 'done', false])
  })

  it('reports the process ending on SessionEnd, but not on /clear', () => {
    const id = newSession()
    receiveHook('UserPromptSubmit', id, {})
    receiveHook('SessionEnd', id, { reason: 'clear' })
    expect(changes.at(-1)).toEqual([id, 'done', true])
    receiveHook('SessionEnd', id, { reason: 'logout' })
    expect(changes.at(-1)).toEqual([id, 'done', false])
  })

  it('ignores events it does not know', () => {
    const id = newSession()
    receiveHook('SomethingNew', id, {})
    expect(getStatus(id)).toBe('done')
  })
})

describe('agents', () => {
  it('keep the card working after the main thread stops', () => {
    const id = newSession()
    receiveHook('UserPromptSubmit', id, {})
    receiveHook('SubagentStart', id, { agent_id: 'a1' })
    receiveHook('Stop', id, {})
    expect(getStatus(id)).toBe('working')
    receiveHook('SubagentStop', id, { agent_id: 'a1' })
    expect(getStatus(id)).toBe('done')
    expect(agentChanges).toEqual([
      [id, 'a1', 'working'],
      [id, 'a1', 'done']
    ])
  })

  it('stay working after stopping with background work that will wake them', () => {
    const id = newSession()
    hasPendingWork.mockReturnValue(true)
    receiveHook('SubagentStart', id, { agent_id: 'a1' })
    receiveHook('SubagentStop', id, { agent_id: 'a1', agent_transcript_path: '/tmp/agent.jsonl' })
    expect(hasPendingWork).toHaveBeenCalledWith('/tmp/agent.jsonl')
    expect(getStatus(id)).toBe('working')
  })

  it('make the card wait while one waits, and an answer goes to that agent', () => {
    const id = newSession()
    receiveHook('SubagentStart', id, { agent_id: 'a1' })
    receiveHook('PermissionRequest', id, { agent_id: 'a1' })
    expect(getStatus(id)).toBe('waiting')
    noteUserInput(id, '\r')
    expect(getStatus(id)).toBe('working')
    expect(agentChanges.at(-1)).toEqual([id, 'a1', 'working'])
  })
})

describe('status line', () => {
  const report = (extra: Record<string, unknown> = {}) => ({
    model: { display_name: 'Opus 5.5' },
    context_window: {
      current_usage: { input_tokens: 1000, cache_creation_input_tokens: 500, cache_read_input_tokens: 1500 },
      context_window_size: 200_000
    },
    cost: { total_cost_usd: 0.1234, total_duration_ms: 5000 },
    ...extra
  })

  it('turns a report into usage', () => {
    const id = newSession()
    receiveHook('StatusLine', id, report(), 1000)
    expect(usages).toEqual([
      [
        id,
        {
          model: 'Opus 5.5',
          contextTokens: 3000,
          contextSize: 200_000,
          contextPercent: 1.5,
          costUsd: 0.1234,
          durationMs: 5000,
          at: 1000
        }
      ]
    ])
  })

  it('prefers the percentage Claude gives', () => {
    const id = newSession()
    receiveHook('StatusLine', id, report({ context_window: { used_percentage: 42, context_window_size: 200_000 } }))
    expect(usages[0][1]).toMatchObject({ contextPercent: 42, contextTokens: undefined })
  })

  it('does not resend a report that only moved the clock', () => {
    const id = newSession()
    receiveHook('StatusLine', id, report(), 1000)
    receiveHook('StatusLine', id, report({ cost: { total_cost_usd: 0.1234, total_duration_ms: 9000 } }), 2000)
    expect(usages).toHaveLength(1)
  })
})

describe('hook commands', () => {
  const commands = () => {
    const settings = JSON.parse(hookSettings())
    return [
      ...Object.values(settings.hooks as Record<string, { hooks: { command: string }[] }[]>).map(
        (h) => h[0].hooks[0].command
      ),
      settings.statusLine.command as string
    ]
  }

  it('registers every hook and the status line, printing nothing', () => {
    setHookEndpoint(4000, 'secret-token', '/tmp/hook.curlrc')
    const settings = JSON.parse(hookSettings())
    expect(Object.keys(settings.hooks)).toEqual(
      expect.arrayContaining(['SessionStart', 'UserPromptSubmit', 'PreToolUse', 'Stop', 'SubagentStop'])
    )
    for (const command of commands()) expect(command).toMatch(/>\/dev\/null 2>&1; exit 0$/)
  })

  it('read the token from the config file when the host takes it in a header', () => {
    setHookEndpoint(4000, 'secret-token', '/tmp/hook.curlrc')
    expect(hookSettings()).not.toContain('secret-token')
    for (const command of commands()) {
      expect(command).toContain('-K "$NODUS_HOOK_CONFIG"')
      expect(command).not.toContain('token=')
    }
    expect(hookEnv('s1')).toEqual({
      NODUS_PORT: '4000',
      NODUS_TOKEN: 'secret-token',
      NODUS_SESSION: 's1',
      NODUS_HOOK_CONFIG: '/tmp/hook.curlrc'
    })
  })

  it('put the token in the URL for an older host', () => {
    setHookEndpoint(4000, 'secret-token')
    expect(hookSettings()).not.toContain('secret-token')
    for (const command of commands()) expect(command).toContain('token=$NODUS_TOKEN')
    expect(hookEnv('s1')).not.toHaveProperty('NODUS_HOOK_CONFIG')
  })
})

describe('hook commands, run by a shell', () => {
  let server: http.Server
  let received: Promise<{ url: string; authorization?: string; body: string }>
  let dir: string

  beforeEach(async () => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'nodus-hooks-'))
    received = new Promise((resolve) => {
      server = http.createServer((req, res) => {
        let body = ''
        req.on('data', (c) => (body += c))
        req.on('end', () => {
          res.writeHead(204).end()
          resolve({ url: req.url ?? '', authorization: req.headers.authorization, body })
        })
      })
    })
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  })

  afterEach(() => {
    server.close()
    fs.rmSync(dir, { recursive: true, force: true })
  })

  /** Runs the Stop hook's command the way Claude Code does, with the payload on stdin. */
  function runStopHook(sessionId: string, payload: string) {
    const command = JSON.parse(hookSettings()).hooks.Stop[0].hooks[0].command
    const child = spawn('sh', ['-c', command], { env: { ...process.env, ...hookEnv(sessionId) } })
    child.stdin.end(payload)
    return new Promise((resolve) => child.on('exit', resolve))
  }

  it('send the token in a header from the config file', async () => {
    const port = (server.address() as AddressInfo).port
    const paths = hostPaths(dir)
    writeHookConfig(paths, 'abc123')
    setHookEndpoint(port, 'abc123', paths.hookConfig)
    await runStopHook('s1', '{"stop":true}')
    expect(await received).toEqual({
      url: '/hook?evt=Stop&sid=s1',
      authorization: 'Bearer abc123',
      body: '{"stop":true}'
    })
  })

  it('send the token in the URL without a config file', async () => {
    const port = (server.address() as AddressInfo).port
    setHookEndpoint(port, 'abc123')
    await runStopHook('s1', '{}')
    expect(await received).toMatchObject({ url: '/hook?evt=Stop&sid=s1&token=abc123', authorization: undefined })
  })
})

describe('hook commands on Windows without Git Bash', () => {
  const platform = process.platform
  let server: http.Server
  let received: Promise<{ url: string; authorization?: string; body: string }>

  beforeEach(async () => {
    Object.defineProperty(process, 'platform', { value: 'win32' })
    vi.mocked(gitBash).mockReturnValue(null)
    received = new Promise((resolve) => {
      server = http.createServer((req, res) => {
        const chunks: Buffer[] = []
        req.on('data', (c: Buffer) => chunks.push(c))
        req.on('end', () => {
          res.writeHead(204).end()
          const body = Buffer.concat(chunks).toString('utf8')
          resolve({ url: req.url ?? '', authorization: req.headers.authorization, body })
        })
      })
    })
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  })

  afterEach(() => {
    Object.defineProperty(process, 'platform', { value: platform })
    vi.mocked(gitBash).mockReset()
    server.close()
  })

  it('run in PowerShell, with the token from the environment', () => {
    setHookEndpoint(4000, 'secret-token', 'C:\\Users\\me\\hook.curlrc')
    const settings = JSON.parse(hookSettings())
    const hooks = [
      ...Object.values(settings.hooks as Record<string, { hooks: { command: string; shell?: string }[] }[]>).map(
        (h) => h[0].hooks[0]
      ),
      settings.statusLine as { command: string; shell?: string }
    ]
    expect(hooks).toHaveLength(11)
    for (const hook of hooks) {
      expect(hook.shell).toBe('powershell')
      expect(hook.command).toContain('"Bearer $env:NODUS_TOKEN"')
      expect(hook.command).toMatch(/exit 0$/)
    }
    expect(JSON.stringify(settings)).not.toContain('secret-token')
  })

  // Needs PowerShell: Windows has it, and a Mac may have PowerShell 7.
  const powershell = platform === 'win32' ? 'powershell.exe' : which('pwsh')
  it.skipIf(!powershell)('post what Claude sends on stdin, byte for byte', async () => {
    const port = (server.address() as AddressInfo).port
    setHookEndpoint(port, 'abc123', 'unused.curlrc')
    const command = JSON.parse(hookSettings()).hooks.Stop[0].hooks[0].command
    const child = spawn(powershell!, ['-NoProfile', '-NonInteractive', '-Command', command], {
      env: { ...process.env, ...hookEnv('s1') },
      windowsHide: true
    })
    let output = ''
    child.stdout.on('data', (d) => (output += d))
    const payload = '{"prompt":"héllo 🎉","last_assistant_message":"done"}'
    child.stdin.end(payload)
    const code = await new Promise((resolve) => child.on('exit', resolve))
    expect(await received).toEqual({ url: '/hook?evt=Stop&sid=s1', authorization: 'Bearer abc123', body: payload })
    expect(code).toBe(0)
    // UserPromptSubmit's output would go into Claude's context.
    expect(output).toBe('')
  }, 30_000)
})

function which(bin: string): string | null {
  for (const dir of (process.env.PATH ?? '').split(path.delimiter)) {
    const candidate = path.join(dir, bin)
    if (dir && fs.existsSync(candidate)) return candidate
  }
  return null
}
