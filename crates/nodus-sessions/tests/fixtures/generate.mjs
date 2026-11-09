// Writes the fixtures beside this file from the TypeScript itself, so the Rust
// tests check the port against what 0.9.4 does rather than against a reading
// of it. Every port, token, path and id here is made up.
//
// The committed fixtures were generated from 0.9.4's sources. The Electron app
// has since left this repo, so to regenerate them, point NODUS_V094 at a
// checkout of tag v0.9.4 and run, from this repo's root, with Node 22:
//
//   git worktree add ../nodus-0.9.4 v0.9.4
//   NODUS_V094=../nodus-0.9.4 node --experimental-strip-types crates/nodus-sessions/tests/fixtures/generate.mjs
//
// It copies src/main/hooks.ts, hostProtocol.ts, discovery.ts and agents.ts to
// a temporary folder, swapping only the imports that need Electron (gitBash,
// isLive) for stubs, and drives them with a fake clock.
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
if (!process.env.NODUS_V094) throw new Error('Set NODUS_V094 to a checkout of tag v0.9.4 (see the top of this file)')
const root = path.resolve(process.env.NODUS_V094)
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'nodus-fixtures-'))
// agents.ts finds ~/.claude/projects from the home folder when it loads.
const home = path.join(work, 'home')
fs.mkdirSync(home)
process.env.HOME = home
process.env.USERPROFILE = home

function copy(name, replacements = []) {
  let source = fs.readFileSync(path.join(root, 'src/main', `${name}.ts`), 'utf8')
  for (const [from, to] of replacements) {
    if (!source.includes(from)) throw new Error(`${name}.ts no longer has: ${from}`)
    source = source.replace(from, to)
  }
  fs.writeFileSync(path.join(work, `${name}.mts`), source)
}

copy('hostProtocol')
copy('hooks', [
  [`import { gitBash } from './env'`, `const gitBash = () => globalThis.__gitBash ?? null`],
  [`from './hostProtocol'`, `from './hostProtocol.mts'`]
])
copy('discovery')
copy('agents', [
  [`from './discovery'`, `from './discovery.mts'`],
  [`from './hooks'`, `from './hooks.mts'`],
  [`import { isLive } from './pty'`, `const isLive = (id: string) => globalThis.__live.has(id)`]
])

globalThis.__live = new Set()
const hooks = await import(pathToFileURL(path.join(work, 'hooks.mts')).href)
const agents = await import(pathToFileURL(path.join(work, 'agents.mts')).href)

// A fake clock and timers, as vi.useFakeTimers gives the TypeScript's tests.
const START = 1_700_000_000_000
let now = START
let nextTimer = 1
const timers = new Map()
globalThis.setTimeout = (fn, ms) => {
  const id = nextTimer++
  timers.set(id, { id, at: now + ms, fn })
  return id
}
globalThis.clearTimeout = (id) => timers.delete(id)
Date.now = () => now
function advance(ms) {
  const end = now + ms
  for (;;) {
    let due
    for (const t of timers.values()) {
      if (t.at <= end && (!due || t.at < due.at || (t.at === due.at && t.id < due.id))) due = t
    }
    if (!due) break
    timers.delete(due.id)
    now = due.at
    due.fn()
  }
  now = end
}

const write = (name, value) => fs.writeFileSync(path.join(here, name), value)
const writeJson = (name, value) => write(name, JSON.stringify(value, null, 2) + '\n')

// ---------------------------------------------------------------------------
// The --settings JSON and the hook environment

const platform = process.platform
const settings = {}
const env = {}
hooks.setHookEndpoint(4000, 'synthetic-token', '/tmp/example/hook.curlrc')
settings.config = hooks.hookSettings()
env.config = hooks.hookEnv('session-1')
hooks.setHookEndpoint(4000, 'synthetic-token')
settings.url = hooks.hookSettings()
env.url = hooks.hookEnv('session-1')
Object.defineProperty(process, 'platform', { value: 'win32' })
globalThis.__gitBash = null
hooks.setHookEndpoint(4000, 'synthetic-token', 'C:\\Users\\example\\hook.curlrc')
settings.powershell = hooks.hookSettings()
hooks.setHookEndpoint(4000, 'synthetic-token')
settings.powershellWithoutConfig = hooks.hookSettings()
// Windows with Git Bash writes POSIX hooks.
globalThis.__gitBash = 'C:\\Program Files\\Git\\bin\\bash.exe'
hooks.setHookEndpoint(4000, 'synthetic-token', 'C:\\Users\\example\\hook.curlrc')
settings.gitBash = hooks.hookSettings()
Object.defineProperty(process, 'platform', { value: platform })
globalThis.__gitBash = null
hooks.setHookEndpoint(0, '')
env.none = hooks.hookEnv('session-1')

write('hook-settings-config.json', settings.config)
write('hook-settings-url.json', settings.url)
write('hook-settings-powershell.json', settings.powershell)
if (settings.powershellWithoutConfig !== settings.powershell) throw new Error('PowerShell settings depend on the config')
if (settings.gitBash !== settings.config) throw new Error('Git Bash settings differ from POSIX ones')
writeJson('hook-env.json', env)

// ---------------------------------------------------------------------------
// The status machine, step by step

let events = []
let pending = false
hooks.configureStatus({
  onChange: (id, status, live) => events.push(['status', id, status, live]),
  onAgentChange: (id, agentId, status) => events.push(['agent', id, agentId, status]),
  onUsage: (id, usage) => events.push(['usage', id, usage]),
  isLive: (id) => globalThis.__live.has(id),
  hasPendingWork: (file) => {
    events.push(['pendingWork', file])
    return pending
  }
})

/** Runs one step; the Rust test runs the same ones. */
function step(s) {
  if ('start' in s) {
    globalThis.__live.add(s.start)
    hooks.markProcess(s.start, true, s.initial)
  } else if ('exit' in s) {
    globalThis.__live.delete(s.exit)
    hooks.markProcess(s.exit, false)
  } else if ('hook' in s) {
    if ('at' in s) hooks.receiveHook(s.hook, s.session, s.payload, s.at)
    else hooks.receiveHook(s.hook, s.session, s.payload)
  } else if ('input' in s) {
    if ('at' in s) hooks.noteUserInput(s.session, s.input, s.at)
    else hooks.noteUserInput(s.session, s.input)
  } else if ('advance' in s) {
    advance(s.advance)
  } else if ('forget' in s) {
    hooks.forgetStatus(s.forget)
  } else if ('settle' in s) {
    hooks.settleAgent(s.settle[0], s.settle[1])
  } else if ('hold' in s) {
    hooks.holdAgent(s.hold[0], s.hold[1])
  } else if ('pending' in s) {
    pending = s.pending
  } else if ('status' in s) {
    events.push(['getStatus', s.status, hooks.getStatus(s.status)])
  } else if ('agent' in s) {
    events.push(['getAgentHook', s.agent[0], s.agent[1], hooks.getAgentHook(s.agent[0], s.agent[1]) ?? null])
  } else if ('usage' in s) {
    events.push(['getUsage', s.usage, hooks.getUsage(s.usage) ?? null])
  } else {
    throw new Error(`unknown step ${JSON.stringify(s)}`)
  }
}

const report = (extra = {}) => ({
  model: { display_name: 'Model A' },
  context_window: {
    current_usage: { input_tokens: 1000, cache_creation_input_tokens: 500, cache_read_input_tokens: 1500 },
    context_window_size: 200000
  },
  cost: { total_cost_usd: 0.1234, total_duration_ms: 5000 },
  ...extra
})

const scenarios = {
  'a turn, start to stop': [
    { start: 's1' },
    { status: 's1' },
    { hook: 'UserPromptSubmit', session: 's1', payload: {} },
    { hook: 'PreToolUse', session: 's1', payload: { tool_name: 'Bash' } },
    { hook: 'PostToolUse', session: 's1', payload: {} },
    { hook: 'Stop', session: 's1', payload: {} },
    { status: 's1' }
  ],
  'a process that starts working': [
    { start: 's1', initial: 'working' },
    { hook: 'SessionStart', session: 's1', payload: {} },
    { status: 's1' },
    { hook: 'Stop', session: 's1', payload: {} },
    { hook: 'SessionStart', session: 's1', payload: { source: 'resume' } },
    { status: 's1' },
    { start: 's1', initial: 'waiting' },
    { hook: 'SessionStart', session: 's1', payload: {} }
  ],
  'permission and questions': [
    { start: 's1' },
    { hook: 'PermissionRequest', session: 's1', payload: {} },
    { hook: 'PostToolUse', session: 's1', payload: {} },
    { hook: 'PreToolUse', session: 's1', payload: { tool_name: 'AskUserQuestion' } },
    { hook: 'PostToolUse', session: 's1', payload: {} },
    { hook: 'Notification', session: 's1', payload: { notification_type: 'permission_prompt' } },
    { hook: 'PostToolUse', session: 's1', payload: {} },
    { hook: 'Notification', session: 's1', payload: { notification_type: 'elicitation_dialog' } },
    { hook: 'PostToolUse', session: 's1', payload: {} },
    { hook: 'Notification', session: 's1', payload: { message: 'Claude needs your PERMISSION to use Bash' } },
    { hook: 'Stop', session: 's1', payload: {} },
    { hook: 'Notification', session: 's1', payload: { notification_type: 'idle_prompt', message: 'Claude is waiting' } },
    { hook: 'Notification', session: 's1', payload: { notification_type: 42, message: null } },
    { hook: 'Notification', session: 's1', payload: { message: 'Permiſſion' } },
    { hook: 'Notification', session: 's1', payload: { message: ['need', 'permission'] } },
    { hook: 'Notification', session: 's1', payload: { notification_type: ['permission_prompt'] } },
    { status: 's1' },
    { hook: 'PreToolUse', session: 's1', payload: { tool_name: 'askuserquestion' } },
    { status: 's1' }
  ],
  'session ends': [
    { start: 's1' },
    { hook: 'UserPromptSubmit', session: 's1', payload: {} },
    { hook: 'SessionEnd', session: 's1', payload: { reason: 'clear' } },
    { hook: 'UserPromptSubmit', session: 's1', payload: {} },
    { hook: 'SessionEnd', session: 's1', payload: { reason: 'logout' } },
    { status: 's1' },
    { hook: 'UserPromptSubmit', session: 's1', payload: {} },
    { status: 's1' },
    { exit: 's1' },
    { status: 's1' }
  ],
  'unknown events and empty payloads': [
    { start: 's1' },
    { hook: 'SomethingNew', session: 's1', payload: {} },
    { hook: 'UserPromptSubmit', session: 's1', payload: null },
    { hook: 'PreToolUse', session: 's1', payload: 7 },
    { hook: 'Stop', session: 's1', payload: 'text' },
    { hook: 'stop', session: 's1', payload: {} },
    { status: 's1' }
  ],
  'keys while waiting': [
    { start: 's1' },
    { hook: 'PermissionRequest', session: 's1', payload: {} },
    { input: '0', session: 's1' },
    { input: 'yes', session: 's1' },
    { input: '\r\r', session: 's1' },
    { input: '\x1b[A', session: 's1' },
    { status: 's1' },
    { input: '1', session: 's1' },
    { hook: 'PermissionRequest', session: 's1', payload: {} },
    { input: '9', session: 's1' },
    { hook: 'PreToolUse', session: 's1', payload: { tool_name: 'AskUserQuestion' } },
    { input: '\r', session: 's1' },
    { status: 's1' },
    { hook: 'Stop', session: 's1', payload: {} },
    { input: '\r', session: 's1' },
    { status: 's1' }
  ],
  'interrupts': [
    { start: 's1' },
    { hook: 'UserPromptSubmit', session: 's1', payload: {} },
    { input: '\x1b', session: 's1' },
    { advance: 1199 },
    { status: 's1' },
    { advance: 1 },
    { status: 's1' },
    { hook: 'UserPromptSubmit', session: 's1', payload: {} },
    { input: '\x03', session: 's1' },
    { advance: 100 },
    { hook: 'PostToolUse', session: 's1', payload: {} },
    { advance: 1200 },
    { status: 's1' },
    { input: '\x1b', session: 's1' },
    { advance: 600 },
    { input: '\x1b', session: 's1' },
    { advance: 600 },
    { status: 's1' },
    { advance: 600 },
    { status: 's1' },
    { hook: 'PermissionRequest', session: 's1', payload: {} },
    { input: '\x03', session: 's1' },
    { advance: 1200 },
    { status: 's1' },
    { input: '\x1b', session: 's1' },
    { advance: 1200 },
    { status: 's1' }
  ],
  'a hook at the same moment as the interrupt': [
    { start: 's1' },
    { hook: 'UserPromptSubmit', session: 's1', payload: {} },
    { input: '\x1b', session: 's1' },
    { hook: 'PostToolUse', session: 's1', payload: {} },
    { advance: 1200 },
    { status: 's1' }
  ],
  'an agent hook after the interrupt': [
    { start: 's1' },
    { hook: 'UserPromptSubmit', session: 's1', payload: {} },
    { input: '\x1b', session: 's1' },
    { advance: 10 },
    { hook: 'PostToolUse', session: 's1', payload: { agent_id: 'a1' } },
    { advance: 1200 },
    { status: 's1' }
  ],
  'the process ends before the interrupt timer': [
    { start: 's1' },
    { hook: 'UserPromptSubmit', session: 's1', payload: {} },
    { input: '\x1b', session: 's1' },
    { exit: 's1' },
    { start: 's1', initial: 'working' },
    { advance: 1500 },
    { status: 's1' }
  ],
  'forgotten while the interrupt timer runs': [
    { start: 's1' },
    { hook: 'UserPromptSubmit', session: 's1', payload: {} },
    { input: '\x1b', session: 's1' },
    { forget: 's1' },
    { advance: 1500 },
    { status: 's1' },
    { usage: 's1' }
  ],
  'agents keep the card working': [
    { start: 's1' },
    { hook: 'UserPromptSubmit', session: 's1', payload: {} },
    { hook: 'SubagentStart', session: 's1', payload: { agent_id: 'a1' } },
    { hook: 'SubagentStart', session: 's1', payload: { agent_id: 'a2', agent_type: 'Explore' } },
    { hook: 'Stop', session: 's1', payload: {} },
    { status: 's1' },
    { hook: 'PreToolUse', session: 's1', payload: { agent_id: 'a1', tool_name: 'Bash' } },
    { hook: 'SubagentStop', session: 's1', payload: { agent_id: 'a1' } },
    { hook: 'SubagentStop', session: 's1', payload: { agent_id: 'a2', agent_transcript_path: 7 } },
    { status: 's1' },
    { agent: ['s1', 'a1'] },
    { agent: ['s1', 'missing'] }
  ],
  'agents woken by background work': [
    { start: 's1' },
    { pending: true },
    { hook: 'SubagentStart', session: 's1', payload: { agent_id: 'a1' } },
    { hook: 'SubagentStop', session: 's1', payload: { agent_id: 'a1', agent_transcript_path: '/tmp/example/agent.jsonl' } },
    { status: 's1' },
    { pending: false },
    { hook: 'SubagentStop', session: 's1', payload: { agent_id: 'a1', agent_transcript_path: '/tmp/example/agent.jsonl' } },
    { status: 's1' },
    { hook: 'SubagentStop', session: 's1', payload: { agent_transcript_path: '/tmp/example/agent.jsonl' } },
    { hook: 'SubagentStart', session: 's1', payload: {} },
    { hook: 'SubagentStart', session: 's1', payload: { agent_id: '' } },
    { status: 's1' }
  ],
  'agents that wait, and answers': [
    { start: 's1' },
    { hook: 'UserPromptSubmit', session: 's1', payload: {} },
    { hook: 'SubagentStart', session: 's1', payload: { agent_id: 'a1' } },
    { advance: 5 },
    { hook: 'SubagentStart', session: 's1', payload: { agent_id: 'a2' } },
    { hook: 'PermissionRequest', session: 's1', payload: { agent_id: 'a2' } },
    { advance: 5 },
    { hook: 'PreToolUse', session: 's1', payload: { agent_id: 'a1', tool_name: 'AskUserQuestion' } },
    { status: 's1' },
    { hook: 'Notification', session: 's1', payload: { notification_type: 'permission_prompt' } },
    { status: 's1' },
    { input: 'x', session: 's1' },
    { input: '\x1b', session: 's1' },
    { agent: ['s1', 'a2'] },
    { input: '\r', session: 's1' },
    { agent: ['s1', 'a1'] },
    { status: 's1' },
    { hook: 'Notification', session: 's1', payload: { agent_id: 'a1', message: 'idle' } },
    { hook: 'Notification', session: 's1', payload: { agent_id: 'a1', notification_type: 'permission_prompt' } },
    { hook: 'Stop', session: 's1', payload: { agent_id: 'a1' } },
    { hook: 'SessionStart', session: 's1', payload: { agent_id: 'a1' } },
    { hook: 'SessionEnd', session: 's1', payload: { agent_id: 'a1' } },
    { hook: 'PermissionRequest', session: 's1', payload: {} },
    { input: '2', session: 's1' },
    { status: 's1' },
    { agent: ['s1', 'a1'] },
    { hook: 'UserPromptSubmit', session: 's1', payload: { agent_id: 'a1' } },
    { advance: 30 },
    { input: '\x03', session: 's1' },
    { advance: 1200 },
    { status: 's1' }
  ],
  'agents waiting at the same moment': [
    { start: 's1' },
    { hook: 'PermissionRequest', session: 's1', payload: { agent_id: 'b' } },
    { hook: 'PermissionRequest', session: 's1', payload: { agent_id: 'a' } },
    { input: '\r', session: 's1' },
    { agent: ['s1', 'b'] },
    { agent: ['s1', 'a'] },
    { hook: 'PermissionRequest', session: 's1', payload: { agent_id: 'a' } },
    { advance: 7 },
    { hook: 'PermissionRequest', session: 's1', payload: { agent_id: 'a' } },
    { agent: ['s1', 'a'] }
  ],
  'settling and holding agents': [
    { start: 's1' },
    { settle: ['s1', 'a1'] },
    { hold: ['s1', 'a1'] },
    { hook: 'SubagentStart', session: 's1', payload: { agent_id: 'a1' } },
    { advance: 3 },
    { hold: ['s1', 'a1'] },
    { settle: ['s1', 'a1'] },
    { agent: ['s1', 'a1'] },
    { settle: ['s1', 'a1'] },
    { advance: 3 },
    { hold: ['s1', 'a1'] },
    { agent: ['s1', 'a1'] },
    { status: 's1' },
    { exit: 's1' },
    { agent: ['s1', 'a1'] },
    { hold: ['s1', 'a1'] }
  ],
  'sessions the app is not running': [
    { hook: 'UserPromptSubmit', session: 'ghost', payload: {} },
    { hook: 'PermissionRequest', session: 'ghost', payload: {} },
    { status: 'ghost' },
    { start: 'ghost' },
    { status: 'ghost' },
    { hook: 'SubagentStart', session: 'other', payload: { agent_id: 'a1' } },
    { input: '\r', session: 'other' },
    { status: 'other' }
  ],
  'replayed history after a relaunch': [
    { start: 's1', initial: 'working' },
    { hook: 'UserPromptSubmit', session: 's1', payload: {}, at: START - 60000 },
    { hook: 'PreToolUse', session: 's1', payload: { tool_name: 'Bash' }, at: START - 50000 },
    { input: '\x1b', session: 's1', at: START - 40000 },
    { hook: 'PostToolUse', session: 's1', payload: {}, at: START - 39000 },
    { hook: 'SubagentStart', session: 's1', payload: { agent_id: 'a1' }, at: START - 30000 },
    { hook: 'PermissionRequest', session: 's1', payload: { agent_id: 'a1' }, at: START - 20000 },
    { input: '\r', session: 's1', at: START - 10000 },
    { agent: ['s1', 'a1'] },
    { hook: 'StatusLine', session: 's1', payload: report(), at: START - 5000 },
    { advance: 1200 },
    { status: 's1' },
    { start: 's2', initial: 'done' },
    { hook: 'UserPromptSubmit', session: 's2', payload: {}, at: START - 9000 },
    { input: '\x03', session: 's2', at: START - 8000 },
    { status: 's2' },
    { advance: 1199 },
    { status: 's2' },
    { advance: 1 },
    { status: 's2' }
  ],
  'usage from the status line': [
    { start: 's1' },
    { hook: 'StatusLine', session: 's1', payload: report(), at: 1000 },
    { hook: 'StatusLine', session: 's1', payload: report({ cost: { total_cost_usd: 0.1234, total_duration_ms: 9000 } }), at: 2000 },
    { usage: 's1' },
    { hook: 'StatusLine', session: 's1', payload: report({ cost: { total_cost_usd: 0.1249, total_duration_ms: 9000 } }), at: 3000 },
    { hook: 'StatusLine', session: 's1', payload: report({ cost: { total_cost_usd: 0.125, total_duration_ms: 9000 } }), at: 4000 },
    { hook: 'StatusLine', session: 's1', payload: report({ model: { id: 'model-b' } }), at: 5000 },
    { hook: 'StatusLine', session: 's1', payload: report({ context_window: { used_percentage: 42, context_window_size: 200000 } }), at: 6000 },
    { hook: 'StatusLine', session: 's1', payload: report({ context_window: { used_percentage: 42, context_window_size: 200000 } }), at: 7000 },
    { advance: 10000 },
    { exit: 's1' },
    { usage: 's1' },
    { advance: 5000 },
    { start: 's1' },
    { hook: 'StatusLine', session: 's1', payload: report({ context_window: { used_percentage: 42, context_window_size: 200000 } }) },
    { advance: 2500 },
    { exit: 's1' },
    { forget: 's1' },
    { usage: 's1' },
    { start: 's1' },
    { exit: 's1' }
  ],
  'usage without a duration': [
    { start: 's1' },
    { hook: 'StatusLine', session: 's1', payload: { model: { display_name: 'Model A' } } },
    { advance: 1000 },
    { exit: 's1' },
    { start: 's1' },
    { hook: 'StatusLine', session: 's1', payload: { model: { display_name: 'Model A' } } },
    { hook: 'StatusLine', session: 's1', payload: null },
    { hook: 'StatusLine', session: 's1', payload: null }
  ],
  'usage for a session the app is not running': [
    { hook: 'StatusLine', session: 'ghost', payload: report(), at: 1000 },
    { usage: 'ghost' },
    { start: 'ghost' },
    { exit: 'ghost' }
  ]
}

const scenarioFixture = []
for (const [name, steps] of Object.entries(scenarios)) {
  // Each scenario starts from nothing.
  events = []
  for (const id of ['s1', 's2', 'ghost', 'other']) {
    hooks.markProcess(id, false)
    hooks.forgetStatus(id)
    globalThis.__live.delete(id)
  }
  advance(10_000)
  now = START
  pending = false
  events = []
  for (const s of steps) step(s)
  scenarioFixture.push({ name, steps, events })
}
writeJson('status-scenarios.json', { start: START, scenarios: scenarioFixture })

// ---------------------------------------------------------------------------
// Status line reports as usage

const usagePayloads = [
  report(),
  report({ context_window: { used_percentage: 42, context_window_size: 200000 } }),
  report({ context_window: { current_usage: null, context_window_size: 200000 } }),
  report({ context_window: { current_usage: [], context_window_size: 200000 } }),
  report({ context_window: { current_usage: { input_tokens: '5', cache_read_input_tokens: 7 }, context_window_size: 0 } }),
  report({ context_window: { current_usage: { input_tokens: 1 }, context_window_size: 3 } }),
  report({ context_window: { current_usage: { input_tokens: 2 }, context_window_size: 3, used_percentage: null } }),
  report({ context_window: { used_percentage: 0, context_window_size: '200000' } }),
  report({ context_window: null }),
  report({ context_window: 'big' }),
  report({ model: null }),
  report({ model: 'Model A' }),
  report({ model: { display_name: '', id: 'model-id' } }),
  report({ model: { display_name: 5 } }),
  report({ model: { display_name: 0, id: false } }),
  report({ model: { display_name: true } }),
  report({ model: { display_name: { name: 'x' } } }),
  report({ model: { display_name: ['a', null, ['b', 'c']] } }),
  report({ model: { display_name: [] } }),
  report({ model: { id: 1.5e21 } }),
  report({ model: { id: 0.000001234 } }),
  report({ cost: null }),
  report({ cost: { total_cost_usd: '0.5' } }),
  report({ cost: { total_cost_usd: 0, total_duration_ms: 0 } }),
  report({ cost: { total_cost_usd: 1e-7, total_duration_ms: 123.456 } }),
  {},
  { model: { display_name: 'Model A' }, context_window: { current_usage: { input_tokens: 1e300, cache_read_input_tokens: 1e300 }, context_window_size: 1e300 } }
]
const usageFixture = []
hooks.configureStatus({
  onChange: () => {},
  onAgentChange: () => {},
  onUsage: (_id, usage) => usageFixture.push({ payload: usagePayloads[usageFixture.length], usage }),
  isLive: () => true,
  hasPendingWork: () => false
})
usagePayloads.forEach((payload, i) => hooks.receiveHook('StatusLine', `usage-${i}`, payload, 1000 + i))
writeJson('usage.json', usageFixture)

// ---------------------------------------------------------------------------
// Subagents: transcripts and meta files read as agents.ts reads them

const lines = (...objects) => objects.map((o) => (typeof o === 'string' ? o : JSON.stringify(o)) + '\n').join('')
const at = (time) => `2025-06-01T${time}Z`
const early = (time) => `2023-11-01T${time}Z`
const text = (t) => ({ type: 'text', text: t })
const tool = (name, input) => ({ type: 'tool_use', id: `toolu_${name}`, name, input })
const said = (timestamp, content, extra = {}) => ({ type: 'assistant', timestamp, message: { role: 'assistant', content }, ...extra })
const asked = (timestamp, content, extra = {}) => ({ type: 'user', timestamp, message: { role: 'user', content }, ...extra })
const longCommand = `git log --oneline ${'--author=example '.repeat(12)}-- src/`
const longReply = `${'The report goes on and on. '.repeat(10)}End.`

const agentFiles = {
  'agent-a1.jsonl': {
    mtime: START - 50_000,
    content: lines(
      asked(at('10:00:00.000'), 'Look into the flaky test'),
      said(at('10:00:05.000'), [text('Running it.'), tool('Bash', { command: 'npm test -- --watch=false', run_in_background: true })]),
      asked(at('10:00:06.000'), [{ type: 'tool_result', tool_use_id: 'toolu_Bash', content: 'started' }], { toolUseResult: { backgroundTaskId: 'bg-1' } }),
      said(at('10:00:07.000'), [text('## Waiting\n\n**Tests** are running in `bg-1`; __I__ will report back.')], { message: { role: 'assistant', stop_reason: 'end_turn', content: [text('## Waiting\n\n**Tests** are running in `bg-1`; __I__ will report back.')] } })
    )
  },
  'agent-a1.meta.json': { mtime: START - 50_000, content: JSON.stringify({ agentType: 'Explore', description: 'Find the flaky test' }) },
  'agent-a2.jsonl': {
    mtime: START - 40_000,
    content: lines(
      asked(at('10:01:00.000'), 'Put a note on the board'),
      said(at('10:01:01.000'), [tool('Read', { file_path: '/repo/src/very/deep/file.ts' })]),
      said(at('10:01:02.000'), [tool('mcp__nodus__create_note', { text: 'x' }), tool('mcp__nodus__', { query: '  ' })]),
      said(at('10:01:03.000'), [tool('SubagentHandback', { message: '# Report\n\nAll done: the note is on the board.' })])
    )
  },
  'agent-a2.meta.json': { mtime: START - 30_000, content: JSON.stringify({ agentType: 'general-purpose', name: 'Note taker', finished: true }) },
  'agent-a3.jsonl': {
    mtime: START - 35_000,
    content: lines({ type: 'assistant', message: { stop_reason: 'end_turn', content: '  A reply with no time  ' } })
  },
  'agent-a3.meta.json': { mtime: START - 35_000, content: JSON.stringify({ stoppedByUser: true, parentAgentId: 'a1', description: '' }) },
  'agent-a4.jsonl': {
    mtime: START - 20_000,
    content:
      lines(
        asked(early('09:00:00.000'), [{ type: 'tool_result', content: 'ok' }], { toolUseResult: { backgroundTaskId: 'bg-7' } }),
        asked(early('09:00:00.500'), [{ type: 'tool_result', content: 'ok' }], { toolUseResult: 'Error: no task' }),
        asked(early('09:00:01.000'), '<task-notification><task-id> bg-7 </task-id><summary>done</summary></task-notification>', { origin: { kind: 'task-notification' } }),
        'not json',
        'null',
        '5',
        '',
        said(early('08:59:59.000'), [tool('Bash', { command: longCommand, description: 'History' })]),
        said(early('09:00:02.000'), [text('Thinking… 🎉 é')]),
        asked(early('09:00:03.000'), [text('more context')], { isMeta: true })
      ) + JSON.stringify(said(early('09:00:04.000'), [text('half writ')])).slice(0, 40)
  },
  'agent-a4.meta.json': { mtime: START - 20_000, content: '[1, 2]' },
  'agent-a5.jsonl': {
    mtime: START - 10_000,
    content: lines(
      said(early('09:10:01.000'), [text('Done.')], { message: { role: 'assistant', stop_reason: 'end_turn', content: [text('Done.')] } }),
      said(early('09:10:02.000'), [tool('Glob', { path: 'lib' })]),
      said(early('09:10:03.000'), [tool('Grep', { pattern: 'TODO', path: 'src' }), tool('Edit', { file_path: '  ', notebook_path: 'nb/analysis.ipynb' })])
    )
  },
  'agent-a5.meta.json': { mtime: START - 10_000, content: '{"agentType": "Plan", "description": "half' },
  'agent-a6.jsonl': {
    mtime: START - 9_000,
    content: lines(said(early('09:20:00.000'), [text('#####\n###### Six\n####### Seven\n   # Indented\n    # Too far\n\t#\tTabbed\n\n \n' + longReply)]))
  },
  'agent-a7.jsonl': {
    mtime: START - 8_000,
    content: lines(said(early('09:30:00.000'), [text('# Only a heading')], { message: { role: 'assistant', stop_reason: 'end_turn', content: [text('# Only a heading')] } }))
  },
  'agent-a7.meta.json': { mtime: START - 8_000, content: '{"agentType": "", "name": "Heading writer", "parentAgentId": "", "stoppedByUser": "yes", "finished": 1}' },
  'agent-a8.jsonl': {
    mtime: START - 7_000,
    content: lines(said(early('09:40:00.000'), [tool('Bash', { command: longCommand, description: 'History' }), { type: 'tool_use', name: 7 }, null]))
  },
  'agent-x.txt': { mtime: START, content: 'not an agent' },
  'agent-.jsonl': { mtime: START, content: '' },
  'agent-a b.jsonl': { mtime: START, content: '' },
  'notes.jsonl': { mtime: START, content: '' }
}

const agentSteps = [
  { files: agentFiles },
  { start: 'S' },
  { hook: 'SubagentStart', session: 'S', payload: { agent_id: 'a1' } },
  { hook: 'SubagentStart', session: 'S', payload: { agent_id: 'a2' } },
  { hook: 'PreToolUse', session: 'S', payload: { agent_id: 'a3', tool_name: 'AskUserQuestion' } },
  { hook: 'SubagentStart', session: 'S', payload: { agent_id: 'a4' } },
  { hook: 'SubagentStop', session: 'S', payload: { agent_id: 'a1', agent_transcript_path: '$DIR/agent-a1.jsonl' } },
  { advance: 1000 },
  { read: 'S' },
  { pendingWork: '$DIR/agent-a1.jsonl' },
  { pendingWork: '$DIR/../subagents/./agent-a1.jsonl' },
  { pendingWork: '$DIR/agent-a2.jsonl' },
  { pendingWork: '$DIR/agent-a1.meta.json' },
  { pendingWork: '$DIR/missing.jsonl' },
  { pendingWork: '/tmp/example/agent-a1.jsonl' },
  { attach: { sessionId: 'S', x: 0, y: 0, archived: false, createdAt: 0, status: 'working', live: true }, lastPromptAt: Date.parse(at('10:00:30.000')) },
  { attach: { sessionId: 'S', x: 0, y: 0, archived: true, createdAt: 0, status: 'done', live: true } },
  { attach: { sessionId: 'S', kind: 'terminal', x: 0, y: 0, archived: false, createdAt: 0, status: 'done', live: true } },
  { attach: { sessionId: 'nobody', x: 0, y: 0, archived: false, createdAt: 0, status: 'done', live: false } },
  // a1's background command reports back and wakes it; it grew, so parsing carries on.
  {
    files: {
      'agent-a1.jsonl': {
        append: true,
        mtime: START + 5_000,
        content: lines(
          asked(at('10:02:00.000'), [text('<task-notification><task-id>bg-1</task-id><summary>Tests passed</summary></task-notification>')]),
          said(at('10:02:01.000'), [tool('Grep', { pattern: 'flaky' })])
        )
      }
    }
  },
  { hook: 'PostToolUse', session: 'S', payload: { agent_id: 'a1' } },
  { pendingWork: '$DIR/agent-a1.jsonl' },
  { read: 'S' },
  // a2 rewritten shorter, and the same size with a new time: both start over.
  {
    files: {
      'agent-a2.jsonl': { mtime: START + 6_000, content: lines(said(at('10:05:00.000'), [tool('Write', { file_path: 'notes/new.md' })])) },
      'agent-a4.jsonl': { mtime: START + 6_000, same: true },
      'agent-a5.meta.json': { mtime: START + 6_000, content: '{"agentType": "Plan", "description": "Plan it", "finished": true}' }
    }
  },
  { hook: 'SubagentStart', session: 'S', payload: { agent_id: 'a5' } },
  { advance: 1000 },
  { read: 'S' },
  { exit: 'S' },
  { read: 'S' }
]

function runAgentSteps(dir) {
  const results = []
  const resolve = (p) => p.replace('$DIR', dir)
  for (const s of agentSteps) {
    if ('files' in s) {
      for (const [name, file] of Object.entries(s.files)) {
        const target = path.join(dir, name)
        if (file.append) fs.appendFileSync(target, file.content)
        else if (!file.same) fs.writeFileSync(target, file.content)
        fs.utimesSync(target, file.mtime / 1000, file.mtime / 1000)
      }
    } else if ('read' in s) {
      results.push(['agents', agents.readAgents('/repo', s.read)])
    } else if ('pendingWork' in s) {
      results.push(['pendingWork', s.pendingWork, agents.hasPendingWork(resolve(s.pendingWork))])
    } else if ('attach' in s) {
      const card = { ...s.attach }
      agents.attachAgents(card, '/repo', s.lastPromptAt)
      results.push(['card', { agentCount: card.agentCount, agents: card.agents }])
    } else {
      const copy = JSON.parse(JSON.stringify(s))
      if (copy.payload?.agent_transcript_path) copy.payload.agent_transcript_path = resolve(copy.payload.agent_transcript_path)
      events = results
      step(copy)
    }
  }
  return results
}

{
  hooks.configureStatus({
    onChange: (id, status, live) => events.push(['status', id, status, live]),
    onAgentChange: (id, agentId, status) => events.push(['agent', id, agentId, status]),
    onUsage: (id, usage) => events.push(['usage', id, usage]),
    isLive: (id) => globalThis.__live.has(id),
    hasPendingWork: agents.hasPendingWork
  })
  now = START
  const dir = path.join(home, '.claude', 'projects', '-repo', 'S', 'subagents')
  fs.mkdirSync(dir, { recursive: true })
  const results = runAgentSteps(dir)
  writeJson('agents.json', { start: START, steps: agentSteps, results })
}

fs.rmSync(work, { recursive: true, force: true })
console.log(`fixtures written to ${path.relative(process.cwd(), here)}`)
