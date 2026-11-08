// Writes the synthetic world (world.mjs) to disk for one app launch:
//
//   <work>/bin/        stand-ins for claude and the login shell, put first on the app's PATH
//   <work>/home/       HOME: ~/.claude/projects/<encoded repo>/ holds the transcripts
//   <work>/repos/<p>/  one small git repository per project
//   <work>/remotes/    a bare remote, so a branch can be ahead of its upstream
//   <work>/userData/   NODUS_USER_DATA: state.json and views/
//   <work>/responses.json  what the claude stand-in answers its -p calls with
//
// Everything is deterministic: fixed ids, times, authors and commit dates, so
// commit hashes and every drawn time are the same on every run and machine.
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  ACTIONS,
  AGENTS,
  AUTHORS,
  BOARDS,
  CAPTURES,
  DAY,
  GROUPS,
  HOUR,
  MIN,
  NOW,
  PROJECTS,
  RESPONSES,
  SESSIONS,
  VIEW_FILES,
  ago,
  groupId,
  hex,
  noteId,
  projectId,
  sectionId,
  sessionId,
  uuid,
  viewId
} from './world.mjs'
import { PAGES } from './pages.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const iso = (t) => new Date(t).toISOString()

/**
 * Where the world goes by default: macOS's Shared folder under /Users, which
 * every account can write to. The app abbreviates any /Users/<name>/ prefix to
 * ~, so the paths it shows read ~/nodus-parity/… on every Mac, and none of
 * them is anyone's home folder.
 */
export const DEFAULT_WORK = path.join(path.sep, 'Users', 'Shared', 'nodus-parity')

/** Claude Code's transcript folder name for a repository: its real path with every other character a dash. */
export const encodeRepoPath = (p) => fs.realpathSync(p).replace(/[^a-zA-Z0-9]/g, '-')

// ---------------------------------------------------------------------------
// Stand-ins

function writeBin(work) {
  const bin = path.join(work, 'bin')
  fs.mkdirSync(bin, { recursive: true })
  // A shebang can't hold a path with spaces, so the scripts run through this link.
  fs.symlinkSync(process.execPath, path.join(bin, 'node'))
  const node = path.join(bin, 'node')
  const claude = fs.readFileSync(path.join(here, 'stand-ins', 'claude.mjs'), 'utf8')
  fs.writeFileSync(path.join(bin, 'claude'), `#!${node}\n${claude}`, { mode: 0o755 })
  fs.copyFileSync(path.join(here, 'stand-ins', 'shell.mjs'), path.join(bin, 'shell.mjs'))
  // $SHELL: `-ilc <script>` and `-c <command>` run in /bin/sh; `-l` alone (a terminal card) is the stand-in screen.
  fs.writeFileSync(
    path.join(bin, 'fakeshell'),
    `#!/bin/sh\nif [ "$#" -ge 2 ]; then exec /bin/sh -c "$2"; fi\nexec "${node}" "${path.join(bin, 'shell.mjs')}"\n`,
    { mode: 0o755 }
  )
}

// ---------------------------------------------------------------------------
// Git

function git(cwd, args, at, who = AUTHORS.avery) {
  const date = `${Math.floor(at / 1000)} +0000`
  execFileSync('git', ['-c', 'commit.gpgsign=false', '-c', 'core.hooksPath=/dev/null', ...args], {
    cwd,
    stdio: 'pipe',
    env: {
      PATH: '/usr/bin:/bin:/opt/homebrew/bin:/usr/local/bin',
      HOME: cwd,
      GIT_CONFIG_GLOBAL: '/dev/null',
      GIT_CONFIG_NOSYSTEM: '1',
      GIT_AUTHOR_NAME: who.name,
      GIT_AUTHOR_EMAIL: who.email,
      GIT_COMMITTER_NAME: who.name,
      GIT_COMMITTER_EMAIL: who.email,
      GIT_AUTHOR_DATE: date,
      GIT_COMMITTER_DATE: date,
      TZ: 'UTC'
    }
  })
}

function write(dir, file, text) {
  fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true })
  fs.writeFileSync(path.join(dir, file), text)
}

const commit = (dir, subject, at, who) => {
  git(dir, ['add', '-A'], at, who)
  git(dir, ['commit', '-q', '-m', subject], at, who)
}

const GRID = `import { useProducts } from '../data/products'
import { ProductCard } from './ProductCard'

export function ProductGrid({ category }: { category: string }) {
  const { products } = useProducts(category)
  return (
    <ul className="grid">
      {products.map((p) => (
        <ProductCard key={p.id} product={p} />
      ))}
    </ul>
  )
}
`

const TAX = `export function lineTax(price: number, rate: number) {
  return Math.round(price * rate * 100) / 100
}

export function orderTax(lines: { price: number; rate: number }[]) {
  return lines.reduce((sum, l) => sum + lineTax(l.price, l.rate), 0)
}
`

function seedRepo(work, key) {
  const dir = path.join(work, 'repos', key)
  fs.mkdirSync(dir, { recursive: true })
  const created = PROJECTS.find((p) => p.key === key).createdAt
  git(dir, ['init', '-q', '-b', 'main'], created)
  write(dir, 'README.md', `# ${key}\n\nA made-up project for the Nodus parity screenshots.\n`)
  write(dir, 'package.json', `{\n  "name": "${key}",\n  "private": true\n}\n`)
  commit(dir, 'Initial commit', created)
  if (key !== 'storefront') {
    write(dir, 'CHANGELOG.md', '# Changelog\n\n- First release\n')
    commit(dir, 'Add a changelog', created + DAY, AUTHORS.sam)
    return dir
  }

  // storefront: a branch two commits ahead of its upstream, with uncommitted changes.
  write(dir, 'src/grid/ProductGrid.tsx', GRID)
  write(dir, 'src/grid/ProductCard.tsx', 'export function ProductCard() {\n  return null\n}\n')
  write(dir, 'src/checkout/tax.ts', TAX)
  write(dir, 'src/utils/price.ts', 'export const formatPrice = (n: number) => `$${n.toFixed(2)}`\n')
  write(dir, 'src/legacy/oldGrid.ts', 'export const OLD_GRID = true\n')
  commit(dir, 'Add product grid and cards', ago(10 * DAY))
  write(dir, 'src/cart/store.ts', 'export const useCartStore = () => null\n')
  commit(dir, 'Move the cart to a Zustand store', ago(2 * DAY), AUTHORS.sam)
  write(dir, 'src/checkout/tax.ts', TAX.replace('rate * 100', 'rate * 100 + Number.EPSILON'))
  commit(dir, 'Round tax per line before summing', ago(26 * HOUR))

  const remote = path.join(work, 'remotes', 'storefront.git')
  fs.mkdirSync(remote, { recursive: true })
  git(remote, ['init', '-q', '--bare', '-b', 'main'], ago(DAY))
  git(dir, ['remote', 'add', 'origin', remote], ago(DAY))
  git(dir, ['checkout', '-q', '-b', 'feat/product-grid'], ago(DAY))
  git(dir, ['push', '-q', '-u', 'origin', 'main', 'feat/product-grid'], ago(DAY))
  write(dir, 'src/grid/ProductGrid.tsx', GRID.replace('const { products }', 'const { products, loading }'))
  commit(dir, 'Add skeleton tiles to the product grid', ago(40 * MIN))
  write(dir, 'src/search/Results.tsx', 'export function Results() {\n  return null\n}\n')
  commit(dir, 'Use the skeleton on search results', ago(15 * MIN), AUTHORS.sam)

  // Uncommitted: modified, added (untracked), deleted and renamed files.
  write(
    dir,
    'src/grid/ProductGrid.tsx',
    GRID.replace('const { products }', 'const { products, loading }').replace(
      '  return (\n    <ul className="grid">',
      '  if (loading) return <ProductSkeleton count={12} />\n  return (\n    <ul className="grid" aria-busy={loading}>'
    ).replace("import { ProductCard } from './ProductCard'", "import { ProductCard } from './ProductCard'\nimport { ProductSkeleton } from './ProductSkeleton'")
  )
  write(dir, 'src/checkout/tax.ts', TAX.replace('return Math.round(price * rate * 100) / 100', 'return price * rate').replace(
    'return lines.reduce((sum, l) => sum + lineTax(l.price, l.rate), 0)',
    'const total = lines.reduce((sum, l) => sum + lineTax(l.price, l.rate), 0)\n  return Math.round(total * 100) / 100'
  ))
  write(
    dir,
    'src/grid/ProductSkeleton.tsx',
    'export function ProductSkeleton({ count }: { count: number }) {\n  return (\n    <ul className="grid" aria-hidden="true">\n      {Array.from({ length: count }, (_, i) => (\n        <li key={i} className="tile is-skeleton" />\n      ))}\n    </ul>\n  )\n}\n'
  )
  fs.rmSync(path.join(dir, 'src/legacy/oldGrid.ts'))
  git(dir, ['mv', 'src/utils/price.ts', 'src/utils/formatPrice.ts'], NOW)
  return dir
}

// ---------------------------------------------------------------------------
// Transcripts

const STEP_MS = 40_000

function absolute(repo, input) {
  const out = { ...input }
  if (out.file) {
    out.file_path = path.join(repo, out.file)
    delete out.file
  }
  return out
}

function toolResult(step) {
  if (step.tool === 'Read') return '     1\texport function ProductGrid() {\n     2\t  …'
  if (step.tool === 'Bash') return 'Tests: 31 passed, 31 total'
  if (step.tool === 'Grep' || step.tool === 'Glob') return 'Found 6 files'
  return 'The file has been updated.'
}

/** The lines of a session's transcript, oldest first. */
function transcriptLines(spec, repo, agentsById) {
  const sid = sessionId(spec.key)
  // Each step is one or two lines; times count back from the last one.
  const steps = spec.steps
  const count = steps.reduce((n, s) => n + (s.user || s.text ? 1 : 2), 0)
  let t = NOW - spec.at - (count - 1) * STEP_MS
  const lines = []
  let parent = null
  const base = (type) => {
    const uuidLine = uuid(`line:${spec.key}:${lines.length}`)
    const line = {
      parentUuid: parent,
      isSidechain: false,
      userType: 'external',
      cwd: repo,
      sessionId: sid,
      version: '2.1.0',
      gitBranch: spec.branch,
      type,
      uuid: uuidLine,
      timestamp: iso(t)
    }
    parent = uuidLine
    t += STEP_MS
    return line
  }
  for (const [i, step] of steps.entries()) {
    if (step.user) {
      lines.push({ ...base('user'), message: { role: 'user', content: step.user } })
    } else if (step.text) {
      lines.push({
        ...base('assistant'),
        message: {
          id: `msg_${hex(`${spec.key}:msg:${i}`, 24)}`,
          type: 'message',
          role: 'assistant',
          model: 'claude-opus-5-5',
          content: [{ type: 'text', text: step.text }],
          stop_reason: 'end_turn'
        }
      })
    } else {
      const toolId = `toolu_${hex(`${spec.key}:tool:${i}`, 24)}`
      let name = step.tool
      let input = absolute(repo, step.input ?? {})
      if (step.agent) {
        const agent = agentsById[step.agent]
        name = 'Agent'
        input = { description: agent.description, subagent_type: agent.agentType, prompt: agent.description }
      }
      lines.push({
        ...base('assistant'),
        message: {
          id: `msg_${hex(`${spec.key}:msg:${i}`, 24)}`,
          type: 'message',
          role: 'assistant',
          model: 'claude-opus-5-5',
          content: [{ type: 'tool_use', id: toolId, name, input }],
          stop_reason: 'tool_use'
        }
      })
      const running = step.agent && agentsById[step.agent].status !== 'done'
      if (running) {
        t += STEP_MS
        continue
      }
      lines.push({
        ...base('user'),
        message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: toolId, content: step.agent ? 'Done.' : toolResult(step) }] }
      })
    }
  }
  lines.push({ type: 'ai-title', aiTitle: spec.title, sessionId: sid })
  return lines
}

function agentLines(agent, repo) {
  const sid = sessionId(agent.session)
  const agentId = agentIdOf(agent.key)
  let t = NOW - agent.startedAgo
  let parent = null
  const lines = []
  const base = (type) => {
    const u = uuid(`agentline:${agent.key}:${lines.length}`)
    const line = {
      parentUuid: parent,
      isSidechain: true,
      userType: 'external',
      cwd: repo,
      sessionId: sid,
      agentId,
      version: '2.1.0',
      gitBranch: 'feat/product-grid',
      type,
      uuid: u,
      timestamp: iso(t)
    }
    parent = u
    t += 20_000
    return line
  }
  lines.push({ ...base('user'), message: { role: 'user', content: agent.description } })
  for (const [i, step] of agent.steps.entries()) {
    const id = `msg_${hex(`${agent.key}:msg:${i}`, 24)}`
    if (step.text) {
      lines.push({
        ...base('assistant'),
        message: { id, type: 'message', role: 'assistant', model: 'claude-opus-5-5', content: [{ type: 'text', text: step.text }], stop_reason: step.end ? 'end_turn' : null }
      })
    } else {
      const toolId = `toolu_${hex(`${agent.key}:tool:${i}`, 24)}`
      lines.push({
        ...base('assistant'),
        message: { id, type: 'message', role: 'assistant', model: 'claude-opus-5-5', content: [{ type: 'tool_use', id: toolId, name: step.tool, input: absolute(repo, step.input) }], stop_reason: 'tool_use' }
      })
      lines.push({ ...base('user'), message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: toolId, content: toolResult(step) }] } })
    }
  }
  return lines
}

export const agentIdOf = (key) => `a${hex(`agent:${key}`, 16)}`

function writeTranscripts(work, repos) {
  const projectsDir = path.join(work, 'home', '.claude', 'projects')
  const agentsById = Object.fromEntries(AGENTS.map((a) => [a.key, a]))
  for (const spec of SESSIONS) {
    const repo = repos[spec.project]
    const dir = path.join(projectsDir, encodeRepoPath(repo))
    fs.mkdirSync(dir, { recursive: true })
    const lines = transcriptLines(spec, repo, agentsById)
    fs.writeFileSync(path.join(dir, `${sessionId(spec.key)}.jsonl`), lines.map((l) => JSON.stringify(l)).join('\n') + '\n')
  }
  for (const agent of AGENTS) {
    const repo = repos[SESSIONS.find((s) => s.key === agent.session).project]
    const dir = path.join(projectsDir, encodeRepoPath(repo), sessionId(agent.session), 'subagents')
    fs.mkdirSync(dir, { recursive: true })
    const id = agentIdOf(agent.key)
    fs.writeFileSync(path.join(dir, `agent-${id}.jsonl`), agentLines(agent, repo).map((l) => JSON.stringify(l)).join('\n') + '\n')
    const meta = { agentType: agent.agentType, description: agent.description, ...(agent.status === 'done' ? { finished: true } : {}) }
    fs.writeFileSync(path.join(dir, `agent-${id}.meta.json`), JSON.stringify(meta))
  }
}

// ---------------------------------------------------------------------------
// Ink

function ellipse({ cx, cy, rx, ry }) {
  const pts = []
  for (let i = 0; i <= 48; i++) {
    const a = (i / 48) * Math.PI * 2 * 1.04 - 0.4
    pts.push(Math.round(cx + rx * Math.cos(a)), Math.round(cy + ry * Math.sin(a)))
  }
  return pts
}

function wave({ x, y, w }) {
  const pts = []
  for (let i = 0; i <= 40; i++) pts.push(Math.round(x + (i / 40) * w), Math.round(y + Math.sin(i / 3) * 9))
  return pts
}

const inkStroke = (s) => ({
  id: uuid(`ink:${s.key}`),
  kind: s.kind,
  color: s.color,
  points: s.circle ? ellipse(s.circle) : s.wave ? wave(s.wave) : s.points,
  size: s.size,
  ...(s.text ? { text: s.text } : {})
})

// ---------------------------------------------------------------------------
// State

export const DEFAULT_SETTINGS = {
  theme: 'light',
  autoUpdate: true,
  notify: true,
  dockBadge: true,
  pinProjectRail: false,
  focusFeature: false,
  focusMode: false,
  capture: true
}

function buildState(work, repos, theme) {
  const action = (a, global) => ({
    id: uuid(`action:${a.key}`),
    name: a.name,
    icon: a.icon,
    description: a.description,
    prompt: a.prompt,
    ...(global ? { global: true } : {}),
    createdAt: ago(7 * DAY)
  })
  const projects = PROJECTS.map((p) => ({
    id: projectId(p.key),
    name: p.key,
    repoPath: repos[p.key],
    color: p.color,
    createdAt: p.createdAt,
    openedAt: p.openedAt,
    ...(p.archived ? { archived: true } : {}),
    ...(p.group ? { groupId: groupId(p.group) } : {}),
    ...(p.focused ? { focused: true } : {}),
    ...(ACTIONS[p.key] ? { actions: ACTIONS[p.key].map((a) => action(a)) } : {})
  }))
  const groups = GROUPS.map((g) => ({
    id: groupId(g.key),
    name: g.name,
    createdAt: g.createdAt,
    ...(g.collapsed ? { collapsed: true } : {})
  }))
  const boards = {}
  for (const [key, b] of Object.entries(BOARDS)) {
    const nodes = {}
    for (const n of b.nodes) {
      const sid = sessionId(n.key)
      const spec = SESSIONS.find((s) => s.key === n.key)
      nodes[sid] = {
        sessionId: sid,
        ...(n.kind ? { kind: n.kind } : {}),
        x: n.x,
        y: n.y,
        archived: false,
        ...(n.parents ? { parents: n.parents.map(sessionId) } : {}),
        createdByApp: true,
        ...(n.section ? { sectionId: sectionId(n.section) } : {}),
        ...(n.expanded ? { expanded: true } : {}),
        ...(n.runOf ? { runOf: uuid(`action:${n.runOf}`) } : {}),
        ...(n.viewOf ? { viewOf: viewId(n.viewOf) } : {}),
        ...(n.autoMode || n.viewOf ? { autoMode: true } : {}),
        createdAt: spec ? NOW - spec.at - 10 * MIN : ago(3 * HOUR)
      }
    }
    const sections = Object.fromEntries(
      (b.sections ?? []).map((s) => [sectionId(s.key), { id: sectionId(s.key), name: s.name, color: s.color, x: s.x, y: s.y, createdAt: s.createdAt }])
    )
    const notes = Object.fromEntries(
      (b.notes ?? []).map((n) => [
        noteId(n.key),
        {
          id: noteId(n.key),
          text: n.text,
          color: n.color,
          x: n.x,
          y: n.y,
          ...(n.w ? { w: n.w, h: n.h } : {}),
          ...(n.session ? { sessionId: sessionId(n.session) } : {}),
          ...(n.from ? { from: sessionId(n.from), fromProject: projectId(n.fromProject), fromTitle: n.fromTitle } : {}),
          createdAt: n.createdAt
        }
      ])
    )
    const views = Object.fromEntries(
      (b.views ?? []).map((v) => {
        const files = VIEW_FILES[v.key]
        const id = viewId(v.key)
        return [
          id,
          {
            id,
            title: v.title,
            description: files.description,
            x: v.x,
            y: v.y,
            ...(v.w ? { w: v.w, h: v.h } : {}),
            ...(v.brain ? { sessionId: sessionId(v.brain) } : {}),
            ...(files.commands ? { commands: files.commands } : {}),
            ...(files.commands?.some((c) => c.approved) ? { allowedRuns: files.commands.filter((c) => c.approved).map((c) => c.run) } : {}),
            version: v.version,
            createdAt: ago(2 * DAY),
            updatedAt: ago(40 * MIN)
          }
        ]
      })
    )
    boards[projectId(key)] = {
      nodes,
      archive: b.archive,
      sections,
      notes,
      views,
      ink: (b.ink ?? []).map(inkStroke),
      viewport: b.viewport,
      hidden: []
    }
  }
  const captures = CAPTURES.map((c) => ({
    id: uuid(`capture:${c.key}`),
    kind: c.kind,
    projectId: projectId(c.project),
    title: c.title,
    createdAt: c.createdAt,
    ...(c.session ? { sessionId: sessionId(c.session) } : {}),
    ...(c.note ? { noteId: noteId(c.note) } : {}),
    ...(c.view ? { viewId: viewId(c.view) } : {})
  }))
  return {
    projects,
    groups,
    boards,
    settings: { ...DEFAULT_SETTINGS, theme },
    actions: ACTIONS.global.map((a) => action(a, true)),
    captures
  }
}

function writeViews(userData) {
  for (const [key, files] of Object.entries(VIEW_FILES)) {
    const dir = path.join(userData, 'views', viewId(key))
    fs.mkdirSync(dir, { recursive: true })
    if (PAGES[key]) fs.writeFileSync(path.join(dir, 'index.html'), PAGES[key])
    if (files.data) fs.writeFileSync(path.join(dir, 'runtime.json'), JSON.stringify({ data: files.data, results: {}, errors: [] }))
  }
}

// ---------------------------------------------------------------------------

/**
 * Writes a fresh world into `work` (removing what was there) and returns where
 * things are. `mutate(state, world)` lets a scene change state.json first.
 */
export function seed(work, { theme = 'light', mutate } = {}) {
  for (const dir of ['bin', 'home', 'repos', 'remotes', 'userData']) fs.rmSync(path.join(work, dir), { recursive: true, force: true })
  fs.mkdirSync(path.join(work, 'userData'), { recursive: true })
  fs.mkdirSync(path.join(work, 'home'), { recursive: true })
  writeBin(work)
  const repos = {}
  for (const p of PROJECTS) repos[p.key] = fs.realpathSync(seedRepo(work, p.key))
  writeTranscripts(work, repos)
  const state = buildState(work, repos, theme)
  const world = { work, repos, userData: path.join(work, 'userData'), home: path.join(work, 'home') }
  mutate?.(state, world)
  fs.writeFileSync(path.join(work, 'userData', 'state.json'), JSON.stringify(state, null, 2))
  writeViews(world.userData)
  fs.writeFileSync(path.join(work, 'responses.json'), JSON.stringify(RESPONSES, null, 2))
  return world
}
