// A stand-in for the `claude` CLI, so the parity harness never runs the real
// one or makes an API call. The seed copies it to <work>/bin/claude.
//
// - `claude --version` answers like Claude Code.
// - `claude -p …` (section names, summaries, action drafts, capture guesses)
//   prints the JSON the app expects, from <work>/responses.json.
// - Anything else is an interactive session: it draws a static, Claude-Code-like
//   screen from the session's transcript (its last prompt, tool calls and reply),
//   turns on bracketed paste like Claude does, and waits on its input. It redraws
//   from scratch on every resize, so the screen never depends on resize timing.
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const args = process.argv.slice(2)
/** Options come before `--`; the prompt after it may hold anything. */
const options = args.includes('--') ? args.slice(0, args.indexOf('--')) : args
const has = (flag) => options.includes(flag)
const value = (flag) => {
  const i = args.indexOf(flag)
  return i >= 0 ? args[i + 1] : undefined
}

function responses() {
  try {
    return JSON.parse(fs.readFileSync(process.env.PARITY_RESPONSES ?? '', 'utf8'))
  } catch {
    return {}
  }
}

function headless() {
  const r = responses()
  const schema = value('--json-schema')
  let out
  if (schema && schema.includes('"reason"')) out = { structured_output: r.guess }
  else if (schema && schema.includes('"icon"')) out = { structured_output: r.draft }
  else if (has('--fork-session')) out = { result: r.summary ?? 'Summary.' }
  else if (has('--system-prompt')) out = { result: r.sectionName ?? 'Untitled' }
  else out = { result: 'OK' }
  const delay = has('--fork-session') ? (r.summaryDelayMs ?? 0) : (r.delayMs ?? 0)
  setTimeout(() => {
    process.stdout.write(JSON.stringify({ type: 'result', is_error: false, ...out }) + '\n')
    process.exit(0)
  }, delay)
}

// ---------------------------------------------------------------------------
// The interactive screen

function encode(dir) {
  let real = dir
  try {
    real = fs.realpathSync(dir)
  } catch {}
  return real.replace(/[^a-zA-Z0-9]/g, '-')
}

/** The last exchange in the session's transcript: what was asked, the tool calls and the reply. */
function lastTurn(sessionId) {
  const file = path.join(os.homedir(), '.claude', 'projects', encode(process.cwd()), `${sessionId}.jsonl`)
  let lines = []
  try {
    lines = fs.readFileSync(file, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l))
  } catch {
    return null
  }
  let prompt = null
  let steps = []
  for (const line of lines) {
    if (line.isSidechain) continue
    const content = line.message?.content
    if (line.type === 'user') {
      if (typeof content === 'string') {
        prompt = content
        steps = []
      }
    } else if (line.type === 'assistant' && Array.isArray(content)) {
      for (const b of content) {
        if (b.type === 'text') steps.push({ text: b.text })
        else if (b.type === 'tool_use') steps.push({ tool: b.name, input: b.input ?? {} })
      }
    }
  }
  return prompt ? { prompt, steps } : null
}

const ESC = '\x1b['
const orange = (s) => `${ESC}38;2;215;119;87m${s}${ESC}39m`
const dim = (s) => `${ESC}2m${s}${ESC}22m`
const gray = (s) => `${ESC}38;5;245m${s}${ESC}39m`
const bold = (s) => `${ESC}1m${s}${ESC}22m`
const green = (s) => `${ESC}38;2;78;186;101m${s}${ESC}39m`

function wrap(text, width, indent = '') {
  const out = []
  for (const para of text.split('\n')) {
    let line = ''
    for (const word of para.split(/\s+/).filter(Boolean)) {
      if (line && line.length + 1 + word.length > width) {
        out.push(line)
        line = word
      } else line = line ? `${line} ${word}` : word
    }
    out.push(line)
  }
  return out.map((l, i) => (i === 0 ? l : indent + l))
}

function toolLine(step) {
  const i = step.input
  const target = i.file_path ? path.basename(i.file_path) : (i.command ?? i.pattern ?? i.description ?? '')
  const result =
    step.tool === 'Read'
      ? 'Read 84 lines'
      : step.tool === 'Edit' || step.tool === 'Write'
        ? 'Updated with 12 additions and 3 removals'
        : step.tool === 'Bash'
          ? 'Done'
          : 'Found 6 files'
  return [`${green('⏺')} ${bold(step.tool)}(${target})`, `  ${gray('⎿')}  ${gray(result)}`]
}

function draw() {
  const cols = Math.max(20, process.stdout.columns || 80)
  const width = Math.min(cols - 4, 96)
  const sessionId = value('--resume') ?? value('--session-id') ?? ''
  const dash = args.indexOf('--')
  const initial = dash >= 0 ? args[dash + 1] : undefined
  const turn = lastTurn(sessionId) ?? { prompt: initial ?? '', steps: [] }
  const name = path.basename(process.cwd())

  const rows = []
  rows.push(`${orange('✻')} ${bold('Claude Code')} ${gray('v2.1.0')}`)
  rows.push(gray(`  Opus 5.5 · ${name}`))
  rows.push('')
  if (turn.prompt) {
    wrap(turn.prompt, width - 2, '  ').forEach((l, i) => rows.push(gray(i === 0 ? `> ${l}` : l)))
    rows.push('')
  }
  if (turn.steps.length) {
    for (const step of turn.steps.slice(-4)) {
      if (step.tool) rows.push(...toolLine(step))
      else for (const [i, l] of wrap(step.text, width - 2, '  ').entries()) rows.push(i === 0 ? `${'⏺'} ${l}` : l)
      rows.push('')
    }
  } else if (turn.prompt) {
    rows.push(`${orange('✻')} ${orange('Pondering…')} ${gray('(esc to interrupt)')}`)
    rows.push('')
  }
  const rule = gray('─'.repeat(cols))
  rows.push(rule, `${gray('>')} ${ESC}7m ${ESC}27m`, rule, gray('  ⏵⏵ accept edits on (shift+tab to cycle)'))

  // Clear the screen and its scrollback, so each draw replaces the last one whole.
  process.stdout.write(`${ESC}?25l${ESC}?2004h${ESC}H${ESC}2J${ESC}3J${rows.join('\r\n')}`)
}

function interactive() {
  if (process.stdin.isTTY) process.stdin.setRawMode(true)
  process.stdin.resume()
  process.stdin.on('data', () => {})
  process.stdout.on('resize', draw)
  draw()
}

if (has('--version') || has('-v')) {
  process.stdout.write('2.1.0 (Claude Code)\n')
  process.exit(0)
} else if (has('-p') || has('--print')) {
  headless()
} else {
  interactive()
}
