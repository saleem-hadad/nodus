// Writes the fixtures beside this file from xterm.js 6 and registry.ts
// themselves, so the Rust tests check nodus-terminal against what 0.9.4 does
// rather than against a reading of it. All text in them is made up.
//
// Run from the repo root with Node 22, after `pnpm install`:
//
//   node crates/nodus-terminal/tests/fixtures/generate.mjs
//
// It bundles xterm's TypeScript sources (node_modules/@xterm/xterm/src, the
// addons' src) with esbuild, and runs:
// - keys.json: every key in a matrix of keys × modifiers × platform through
//   registry.ts's real custom key handler and xterm's evaluateKeyboardEvent,
//   following CoreBrowserTerminal._keyDown;
// - screens.json: output (Claude-Code-like UIs, wide characters, emoji,
//   erasing, scrolling, the alternate screen) through xterm's own parser and
//   buffer with the Unicode 11 addon active: every line's cells and the cursor;
// - replies.json: the queries programs send and what xterm answers;
// - widths.json: the Unicode 11 width of every code point, as ranges;
// - links.json: the web-links addon's matches on sample lines;
// - registry.json: scenarios run against the real registry.ts with a stand-in
//   terminal and session API: every call it makes, in order.
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath, pathToFileURL } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const root = path.resolve(here, '../../../..')
const modules = path.join(root, 'node_modules')
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'nodus-terminal-fixtures-'))
const require = createRequire(path.join(modules, '.pnpm/node_modules/_.js'))
const esbuild = require('esbuild')

const tsconfigRaw = { compilerOptions: { experimentalDecorators: true, useDefineForClassFields: false } }

async function bundle(entry, out, alias = {}) {
  await esbuild.build({
    entryPoints: [entry],
    bundle: true,
    platform: 'node',
    format: 'esm',
    target: 'node22',
    outfile: path.join(work, out),
    nodePaths: [path.join(modules, '@xterm/xterm/src')],
    alias,
    tsconfigRaw,
    logLevel: 'error'
  })
  return path.join(work, out)
}

const xtermFile = await bundle(path.join(here, 'harness/xterm.ts'), 'xterm.mjs', {
  '@xterm/addon-unicode11': path.join(modules, '@xterm/addon-unicode11'),
  '@xterm/addon-web-links': path.join(modules, '@xterm/addon-web-links')
})
const xterm = await import(pathToFileURL(xtermFile).href)

const fakes = path.join(here, 'harness')
const registryFile = await bundle(path.join(root, 'src/renderer/src/terminal/registry.ts'), 'registry.mjs', {
  '@xterm/xterm': path.join(fakes, 'fake-xterm.ts'),
  '@xterm/addon-fit': path.join(fakes, 'fake-addons.ts'),
  '@xterm/addon-webgl': path.join(fakes, 'fake-addons.ts'),
  '@xterm/addon-unicode11': path.join(fakes, 'fake-addons.ts'),
  '@xterm/addon-web-links': path.join(fakes, 'fake-addons.ts'),
  '@renderer/platform': path.join(root, 'src/renderer/src/platform.ts'),
  '@shared/types': path.join(root, 'src/shared/types.ts')
})

function write(name, value) {
  fs.writeFileSync(path.join(here, name), JSON.stringify(value, null, 1) + '\n')
}

// registry.ts's TERMINAL_THEME, read from the source.
const registrySource = fs.readFileSync(path.join(root, 'src/renderer/src/terminal/registry.ts'), 'utf8')
const themeBlock = registrySource.match(/TERMINAL_THEME: ITheme = \{([\s\S]*?)\n\}/)[1]
const theme = Object.fromEntries([...themeBlock.matchAll(/(\w+): '#([0-9a-f]{6})/g)].map((m) => [m[1], m[2]]))
const rgb = (hex) => [parseInt(hex.slice(0, 2), 16), parseInt(hex.slice(2, 4), 16), parseInt(hex.slice(4, 6), 16)]
const ANSI_NAMES = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white']
const themeColors = {
  ansi: [...ANSI_NAMES, ...ANSI_NAMES.map((n) => 'bright' + n[0].toUpperCase() + n.slice(1))].map((n) => rgb(theme[n])),
  foreground: rgb(theme.foreground),
  background: rgb(theme.background),
  cursor: rgb(theme.cursor)
}

// ---------------------------------------------------------------------------
// Fake DOM, window, session API and clock for registry.ts
// ---------------------------------------------------------------------------

let log = []
globalThis.__log = (entry) => log.push(entry)
globalThis.__terminals = []

function element() {
  const el = {
    style: {},
    className: '',
    children: [],
    parentElement: null,
    isConnected: true,
    listeners: {},
    setAttribute() {},
    addEventListener(type, fn) {
      el.listeners[type] = fn
    },
    appendChild(child) {
      if (child.parentElement) {
        child.parentElement.children = child.parentElement.children.filter((c) => c !== child)
      }
      child.parentElement = el
      child.isConnected = true
      el.children.push(child)
      // Tag the terminal with its session for the log.
    },
    remove() {
      if (el.parentElement) el.parentElement.children = el.parentElement.children.filter((c) => c !== el)
      el.parentElement = null
      el.isConnected = false
    }
  }
  return el
}

let now = 0
let timers = new Map()
let nextTimer = 1
const fakeWindow = {
  setTimeout(fn, ms) {
    const id = nextTimer++
    timers.set(id, { at: now + ms, fn })
    return id
  },
  clearTimeout(id) {
    timers.delete(id)
  },
  open(url) {
    log.push(['window-open', url])
  }
}
async function advance(ms) {
  const until = now + ms
  for (;;) {
    const due = [...timers.entries()].filter(([, t]) => t.at <= until).sort((a, b) => a[1].at - b[1].at)[0]
    if (!due) break
    timers.delete(due[0])
    now = due[1].at
    due[1].fn()
    await settle()
  }
  now = until
}
async function settle() {
  for (let i = 0; i < 5; i++) await new Promise((r) => setImmediate(r))
}

let opens = []
let handlers = {}
const api = {
  sessions: {
    open(projectId, sessionId, cols, rows) {
      log.push(['open', projectId, sessionId, cols, rows])
      let resolve, reject
      const promise = new Promise((res, rej) => ((resolve = res), (reject = rej)))
      opens.push({ resolve, reject })
      return promise
    },
    write(sessionId, data) {
      log.push(['write', sessionId, data])
    },
    resize(sessionId, cols, rows) {
      log.push(['resize', sessionId, cols, rows])
    }
  },
  on: {
    ptyData(fn) {
      handlers.data = fn
    },
    ptyExit(fn) {
      handlers.exit = fn
    }
  },
  menuCommand(cmd) {
    log.push(['menu', cmd])
    return Promise.resolve()
  },
  pathForFile(file) {
    return file.path
  }
}
fakeWindow.api = api
globalThis.window = fakeWindow
globalThis.document = { createElement: element, body: element() }

let instance = 0
async function loadRegistry(platform) {
  Object.defineProperty(globalThis, 'navigator', {
    value: {
      platform: platform === 'mac' ? 'MacIntel' : 'Win32',
      clipboard: {
        writeText(text) {
          log.push(['clipboard', text])
          return Promise.resolve()
        }
      }
    },
    configurable: true
  })
  globalThis.__terminals = []
  timers = new Map()
  now = 0
  opens = []
  handlers = {}
  return import(pathToFileURL(registryFile).href + '?' + instance++)
}

// ---------------------------------------------------------------------------
// keys.json
// ---------------------------------------------------------------------------

const NAMED = [
  ['Backspace', 'Backspace', 8],
  ['Tab', 'Tab', 9],
  ['Enter', 'Enter', 13],
  ['Escape', 'Escape', 27],
  [' ', 'Space', 32],
  ['ArrowLeft', 'ArrowLeft', 37],
  ['ArrowUp', 'ArrowUp', 38],
  ['ArrowRight', 'ArrowRight', 39],
  ['ArrowDown', 'ArrowDown', 40],
  ['Home', 'Home', 36],
  ['End', 'End', 35],
  ['PageUp', 'PageUp', 33],
  ['PageDown', 'PageDown', 34],
  ['Insert', 'Insert', 45],
  ['Delete', 'Delete', 46],
  ...Array.from({ length: 12 }, (_, i) => [`F${i + 1}`, `F${i + 1}`, 112 + i]),
  ['Enter', 'NumpadEnter', 13],
  ['Shift', 'ShiftLeft', 16],
  ['Process', 'KeyA', 229]
]
// [unshifted, shifted, code, keyCode]
const CHARS = [
  ...'abcdefghijklmnopqrstuvwxyz'.split('').map((c) => [c, c.toUpperCase(), 'Key' + c.toUpperCase(), c.toUpperCase().charCodeAt(0)]),
  ...'0123456789'.split('').map((d, i) => [d, ')!@#$%^&*('[i], 'Digit' + d, 48 + i]),
  ['-', '_', 'Minus', 189],
  ['=', '+', 'Equal', 187],
  ['[', '{', 'BracketLeft', 219],
  [']', '}', 'BracketRight', 221],
  ['\\', '|', 'Backslash', 220],
  [';', ':', 'Semicolon', 186],
  ["'", '"', 'Quote', 222],
  [',', '<', 'Comma', 188],
  ['.', '>', 'Period', 190],
  ['/', '?', 'Slash', 191],
  ['`', '~', 'Backquote', 192],
  ['7', '7', 'Numpad7', 103],
  // Russian and Arabic layouts: the key is the layout's letter, the key code the Latin one.
  ['с', 'С', 'KeyC', 67],
  ['м', 'М', 'KeyV', 86],
  ['ф', 'Ф', 'KeyA', 65],
  ['ц', 'Ц', 'KeyW', 87],
  ['е', 'Е', 'KeyT', 84],
  ['л', 'Л', 'KeyK', 75],
  ['ب', 'ب', 'KeyF', 70]
]
// What ⌥ makes of a key on a US Mac layout (dead keys for e, i, n, u, `).
const MAC_OPTION = { a: 'å', b: '∫', c: 'ç', d: '∂', e: 'Dead', f: 'ƒ', g: '©', i: 'Dead', k: '˚', n: 'Dead', o: 'ø', q: 'œ', s: 'ß', t: '†', u: 'Dead', v: '√', w: '∑', z: 'Ω', 1: '¡', 2: '™', '`': 'Dead' }

const COMBOS = []
for (let bits = 0; bits < 16; bits++) {
  COMBOS.push({ shift: !!(bits & 1), alt: !!(bits & 2), ctrl: !!(bits & 4), meta: !!(bits & 8) })
}

function keyFor(platform, [plain, shifted], m) {
  if (platform === 'mac' && m.alt && !m.ctrl && !m.meta && MAC_OPTION[plain] !== undefined) {
    const k = MAC_OPTION[plain]
    return m.shift && k !== 'Dead' ? k.toUpperCase() : k
  }
  return m.shift ? shifted : plain
}

function makeEvent(key, code, keyCode, m, altGraph = false) {
  return {
    type: 'keydown',
    key,
    code,
    keyCode,
    shiftKey: m.shift,
    altKey: m.alt,
    ctrlKey: m.ctrl,
    metaKey: m.meta,
    getModifierState: (name) => name === 'AltGraph' && altGraph,
    preventDefault() {
      this.defaultPrevented = true
    },
    defaultPrevented: false
  }
}

// CoreBrowserTerminal._keyDown after the custom handler let the key through.
function xtermKeyDown(ev, state, platform, appCursor) {
  const isMac = platform === 'mac'
  const isWindows = platform === 'win'
  if (ev.keyCode === 229) return 'composing'
  if (ev.key === 'Dead' || ev.key === 'AltGraph') state.dead = true
  const result = xterm.evaluateKeyboardEvent(ev, appCursor, isMac, false)
  if (result.type === 2) return 'handled:page-up'
  if (result.type === 3) return 'handled:page-down'
  const selectAll = result.type === 1
  const third =
    (isMac && ev.altKey && !ev.ctrlKey && !ev.metaKey) ||
    (isWindows && ev.altKey && ev.ctrlKey && !ev.metaKey) ||
    (isWindows && ev.getModifierState('AltGraph'))
  if (third && (!ev.keyCode || ev.keyCode > 47)) return selectAll ? 'unhandled:select-all' : 'unhandled'
  if (!result.key) return result.cancel ? 'handled' : selectAll ? 'unhandled:select-all' : 'unhandled'
  if (ev.key && !ev.ctrlKey && !ev.altKey && !ev.metaKey && ev.key.length === 1) {
    const c = ev.key.charCodeAt(0)
    if (c >= 65 && c <= 90) return 'unhandled'
  }
  if (state.dead) {
    state.dead = false
    return result.cancel ? 'handled' : 'unhandled'
  }
  return 'send:' + result.key
}

async function keyCase(registry, term, sessionId, ev, platform, appCursor, selection) {
  term.setSelection(selection ? 'SEL' : '')
  log = []
  const passes = term.customKeyHandler(ev)
  await settle()
  const calls = log
  log = []
  if (passes) return xtermKeyDown(ev, {}, platform, appCursor)
  // The custom handler kept it: what did it do?
  const did = calls.map((c) => c[0])
  if (calls.some((c) => c[0] === 'write')) return 'direct:' + calls.find((c) => c[0] === 'write')[2]
  if (did.includes('clipboard')) return did.includes('term-clear-selection') ? 'copy+clear' : 'copy'
  if (did.includes('menu')) return 'menu:' + calls.find((c) => c[0] === 'menu')[1]
  if (did.includes('term-clear')) return 'handled:clear'
  if (did.includes('term-select-all')) return 'handled:select-all'
  if (did.includes('term-clear-selection')) return 'handled'
  if (ev.defaultPrevented) return 'handled'
  // Ctrl+V off a Mac: the browser's own paste goes to the terminal.
  if (platform === 'win' && ev.ctrlKey && !ev.altKey && !ev.metaKey && registry && /^v$/i.test(shortcut(ev))) return 'paste'
  return 'propagate'
}

function shortcut(e) {
  if (/^[a-z]$/i.test(e.key) || e.keyCode < 65 || e.keyCode > 90) return e.key.toLowerCase()
  return String.fromCharCode(e.keyCode).toLowerCase()
}

const NAV = new Set([33, 34, 35, 36, 37, 38, 39, 40, 0])
const keyCases = []
for (const platform of ['mac', 'win']) {
  for (const kind of ['claude', 'terminal']) {
    const registry = await loadRegistry(platform)
    const container = element()
    container.size = [80, 24]
    registry.attach('s1', 'p1', container, kind).catch(() => {})
    await settle()
    const term = globalThis.__terminals.at(-1)
    term.sessionId = 's1'
    const run = async (key, code, keyCode, m, extra = {}) => {
      const variants = [{ app: false, sel: false }]
      if (NAV.has(keyCode)) variants.push({ app: true, sel: false })
      if (['c', 'C', 'с', 'С', 'k', 'a'].includes(key) && (m.ctrl || m.meta)) variants.push({ app: false, sel: true })
      for (const v of variants) {
        const ev = makeEvent(key, code, keyCode, m, extra.altGraph)
        const outcome = await keyCase(registry, term, 's1', ev, platform, v.app, v.sel)
        keyCases.push({ platform, kind, key, code, keyCode, ...m, altGraph: !!extra.altGraph, app: v.app, sel: v.sel, outcome })
      }
    }
    for (const m of COMBOS) {
      // The session kind only changes Enter.
      if (kind === 'terminal') {
        await run('Enter', 'Enter', 13, m)
        continue
      }
      for (const [key, code, keyCode] of NAMED) await run(key, code, keyCode, m)
      for (const c of CHARS) await run(keyFor(platform, c, m), c[2], c[3], m)
    }
    if (kind === 'claude' && platform === 'win') {
      // AltGr+Q on a German layout types @.
      await run('@', 'KeyQ', 81, { shift: false, alt: true, ctrl: true, meta: false }, { altGraph: true })
      // A dead key on US-International, then Alt with a dead key.
      await run('Dead', 'Quote', 222, COMBOS[0])
      await run('Dead', 'KeyN', 78, COMBOS[2])
    }
  }
}
// One case per line: platform, kind, key, code, keyCode, modifiers (S A C M),
// AltGraph, application cursor mode, a selection, and what happened.
fs.writeFileSync(
  path.join(here, 'keys.json'),
  '[\n' +
    keyCases
      .map((c) => {
        const mods = (c.shift ? 'S' : '') + (c.alt ? 'A' : '') + (c.ctrl ? 'C' : '') + (c.meta ? 'M' : '')
        return JSON.stringify([c.platform, c.kind, c.key, c.code, c.keyCode, mods, c.altGraph, c.app, c.sel, c.outcome])
      })
      .join(',\n') +
    '\n]\n'
)

// ---------------------------------------------------------------------------
// screens.json and replies.json
// ---------------------------------------------------------------------------

function screen(name, cols, rows, chunks) {
  const t = new xterm.Headless(cols, rows, themeColors)
  const replies = chunks.map((c) => t.feed(c))
  const d = t.dump()
  return { name, cols, rows, chunks, replies, cursor: d.cursor, ybase: d.ybase, lines: d.lines }
}

const ESC = '\x1b'
const claudeBox = (w) => '╭' + '─'.repeat(w - 2) + '╮\r\n│ > ' + 'try "fix the build"'.padEnd(w - 5) + '│\r\n╰' + '─'.repeat(w - 2) + '╯'
const screens = [
  screen('plain text wraps', 10, 4, ['hello world, this wraps around\r\nnext']),
  screen('wide characters wrap whole', 7, 4, ['ab中文字cd', '\r\n中文中文中']),
  screen('emoji by unicode 11', 12, 4, ['a😀b🥲c🤻d', '\r\n❤️x👍🏽y', '\r\n👨‍👩‍👧z']),
  screen('combining and joining marks', 10, 3, ['éà⃝', '\r\nक्षx', '\r\n각!']),
  screen('newer marks are a cell in unicode 11', 10, 2, ['a᷺᪰b\u{1e94a}c']),
  screen('claude code ui', 40, 8, [
    `${ESC}[?2026h${ESC}[?25l⏺ Read(src/main.rs)\r\n  ⎿  Read 42 lines\r\n\r\n${ESC}[38;5;174m✻${ESC}[39m Thinking… ${ESC}[2m(esc to interrupt)${ESC}[22m\r\n`,
    claudeBox(40),
    `${ESC}[?2026l`,
    `${ESC}[2K${ESC}[1A${ESC}[2K${ESC}[1A${ESC}[2K${ESC}[G✳ Thinking…`
  ]),
  screen('erase in display keeps history out', 10, 3, ['one\r\ntwo\r\nthree\r\nfour', `${ESC}[2J${ESC}[Hx`]),
  screen('erase above from the second line', 6, 3, ['aaaa\r\nbbbb\r\ncccc', `${ESC}[2;3H${ESC}[1J`]),
  screen('erase scrollback', 6, 2, ['1\r\n2\r\n3\r\n4', `${ESC}[3J`]),
  screen('cursor moves and line erases', 10, 4, [`abcdefghij${ESC}[2;5Hxy${ESC}[1;3H${ESC}[K${ESC}[3;8Hz${ESC}[1K${ESC}[4;1H${ESC}[5@q`]),
  screen('insert and delete', 8, 2, [`abcdef${ESC}[1;3H${ESC}[2P${ESC}[4h12${ESC}[4l${ESC}[2X`]),
  screen('tabs', 20, 2, ['a\tb\tc\r\n\t\tx']),
  screen('scroll region', 6, 4, [`1\r\n2\r\n3\r\n4${ESC}[2;3r${ESC}[3;1H\n\nx${ESC}[r`]),
  screen('alternate screens', 8, 3, ['main', `${ESC}[?1049halt1${ESC}[?1049l`, `${ESC}[?47halt2${ESC}[2;2H`, `${ESC}[?47l`]),
  screen('reverse index and insert lines', 6, 4, [`a\r\nb\r\nc${ESC}[H${ESC}M${ESC}[2;1H${ESC}[2L${ESC}[3M`]),
  screen('wrap pending cursor', 5, 2, ['abcde']),
  screen('no wrap mode', 5, 2, [`${ESC}[?7labcdefg${ESC}[?7h`]),
  screen('soft reset', 8, 3, [`${ESC}[2;3r${ESC}[?6h${ESC}[2;2Hx${ESC}[!py`]),
  screen('full reset', 8, 3, [`abc${ESC}[?1h${ESC}cxy`]),
  screen('osc 8 hyperlink text', 30, 2, [`${ESC}]8;;https://example.com/docs${ESC}\\docs${ESC}]8;;${ESC}\\ and more`]),
  screen('decaln', 4, 2, [`${ESC}#8`]),
  screen('backspace and carriage return', 8, 2, ['abc\b\bX\rY']),
  screen('rep', 8, 2, [`a${ESC}[3bz`])
]
write('screens.json', screens)

const queries = [
  ['da1', [`${ESC}[c`, `${ESC}[0c`, `${ESC}[1c`]],
  ['da2', [`${ESC}[>c`, `${ESC}[>0c`, `${ESC}[>1c`]],
  ['da3 and others', [`${ESC}[=c`, `${ESC}[?c`]],
  ['dsr', [`${ESC}[5n`, `ab${ESC}[6n`, `${ESC}[3;4H${ESC}[6n`, `${ESC}[?6n`, `${ESC}[?5n`, `${ESC}[15n`]],
  ['cpr while a wrap is pending', ['12345', `${ESC}[6n`, `${ESC}[?6n`]],
  ['decrqm ansi', [2, 4, 12, 20, 1, 3].map((p) => `${ESC}[${p}$p`)],
  ['decrqm ansi after insert', [`${ESC}[4h${ESC}[4$p`]],
  [
    'decrqm private',
    [1, 3, 6, 7, 8, 9, 12, 25, 45, 47, 66, 67, 1000, 1002, 1003, 1004, 1005, 1006, 1007, 1015, 1016, 1042, 1047, 1048, 1049, 2004, 2026, 9999].map(
      (p) => `${ESC}[?${p}$p`
    )
  ],
  [
    'decrqm private after setting',
    [
      `${ESC}[?1h${ESC}[?1$p`,
      `${ESC}[?1004h${ESC}[?1004$p`,
      `${ESC}[?2004h${ESC}[?2004$p`,
      `${ESC}[?2026h${ESC}[?2026$p${ESC}[?2026l`,
      `${ESC}[?1002h${ESC}[?1006h${ESC}[?1002$p${ESC}[?1006$p${ESC}[?1000$p`,
      `${ESC}[?9h${ESC}[?9$p${ESC}[?1016h${ESC}[?1016$p`,
      `${ESC}[?12l${ESC}[?12$p`,
      `${ESC}[?25l${ESC}[?25$p`,
      `${ESC}[?7l${ESC}[?7$p`,
      `${ESC}[?6h${ESC}[?6$p`,
      `${ESC}[?45h${ESC}[?45$p`,
      `${ESC}[?66h${ESC}[?66$p`,
      `${ESC}[?1049h${ESC}[?1049$p${ESC}[?47$p`,
      `${ESC}[?1049l${ESC}[?1049$p`
    ]
  ],
  [
    'colour queries',
    [
      `${ESC}]10;?\x07`,
      `${ESC}]11;?${ESC}\\`,
      `${ESC}]12;?\x07`,
      `${ESC}]4;1;?\x07`,
      `${ESC}]4;208;?\x07`,
      `${ESC}]4;255;?\x07`,
      `${ESC}]10;?;?\x07`
    ]
  ],
  [
    'colour changes',
    [
      `${ESC}]11;rgb:01/02/03\x07${ESC}]11;?\x07`,
      `${ESC}]111\x07${ESC}]11;?\x07`,
      `${ESC}]4;1;#102030\x07${ESC}]4;1;?\x07`,
      `${ESC}]104\x07${ESC}]4;1;?\x07`,
      `${ESC}]10;#aabbcc\x07${ESC}]104\x07${ESC}]10;?\x07`
    ]
  ],
  ['window ops are off', [14, 16, 18, 19, 20, 21, 22, 23, 8].map((p) => `${ESC}[${p}t`)],
  ['no kitty keyboard', [`${ESC}[?u`, `${ESC}[>1u${ESC}[?u`, `${ESC}[=1;1u`]],
  ['no xtversion or modifyOtherKeys', [`${ESC}[>q`, `${ESC}[>0q`, `${ESC}[?4m`, `${ESC}[>4;2m`]],
  ['no osc 52', [`${ESC}]52;c;?\x07`, `${ESC}]52;c;aGVsbG8=\x07`]],
  [
    'decrqss',
    [`${ESC}P$qm${ESC}\\`, `${ESC}P$qr${ESC}\\`, `${ESC}[2;3r${ESC}P$qr${ESC}\\`, `${ESC}P$q q${ESC}\\`, `${ESC}P$q"q${ESC}\\`, `${ESC}P$q"p${ESC}\\`, `${ESC}P$qx${ESC}\\`, `${ESC}[?12l${ESC}P$q q${ESC}\\`]
  ],
  ['xtgettcap is not answered', [`${ESC}P+q544e${ESC}\\`]],
  ['queries split across writes', [`${ESC}[`, '?2026$', 'p', `${ESC}]1`, '1;?', '\x07']]
]
write(
  'replies.json',
  queries.map(([name, chunks]) => {
    const t = new xterm.Headless(20, 5, themeColors)
    return { name, chunks, replies: chunks.map((c) => t.feed(c)) }
  })
)

// ---------------------------------------------------------------------------
// widths.json
// ---------------------------------------------------------------------------

const widthRanges = []
for (let cp = 0; cp <= 0x10ffff; cp++) {
  const w = xterm.unicode11.wcwidth(cp)
  const last = widthRanges.at(-1)
  if (last && last[2] === w && last[1] === cp - 1) last[1] = cp
  else widthRanges.push([cp, cp, w])
}
write('widths.json', widthRanges)

// ---------------------------------------------------------------------------
// links.json
// ---------------------------------------------------------------------------

const linksSource = fs.readFileSync(path.join(modules, '@xterm/addon-web-links/src/WebLinksAddon.ts'), 'utf8')
const regexSource = linksSource.match(/const strictUrlRegex = \/(.*)\/;/)[1]
const strictUrlRegex = new RegExp(regexSource)
const linkLines = [
  'see https://example.com/docs for more',
  'trailing punctuation https://a.example/x). and (https://b.example/y).',
  'quoted "https://c.example/path?q=1&r=2" and \'http://d.example\'',
  'HTTPS://UPPER.EXAMPLE/Path and Https://mixed.example',
  'user info https://user:pass@e.example:8080/p#frag',
  'not a url: https:// nor ftp://f.example nor https://exa mple',
  'ports and paths https://g.example:3000/a/b/c.html?x=y',
  'brackets https://h.example/[x] and https://i.example/{y} and https://j.example/a|b',
  'unicode https://ü.example/päth and https://k.example/日本',
  'ends with comma https://l.example/a, then text',
  'ipv6 http://[::1]:8000/x and localhost http://localhost:5173/',
  'a very long url that wraps https://wrap.example/' + 'segment/'.repeat(8) + 'end and after',
  'tilde https://m.example/~user/ and hash https://n.example/#/route',
  'bad host https://-bad-.example/ and https://exa_mple.com/',
  'percent https://o.example/a%20b and spaces%20https://p.example',
  'double https://q.example/https://r.example'
]
const osc = (uri, text) => `${ESC}]8;;${uri}${ESC}\\${text}${ESC}]8;;${ESC}\\`
const oscLines = [
  osc('https://example.com/docs', 'docs') + ' ' + osc('file:///tmp/x', 'file') + ' ' + osc('https://example.com/b', 'https://example.com/b'),
  'x' + osc('https://a.example', 'ab') + osc('https://b.example', 'cd') + 'y',
  `${ESC}]8;id=1;https://same.example${ESC}\\one${ESC}]8;;${ESC}\\ ${ESC}]8;id=1;https://same.example${ESC}\\two${ESC}]8;;${ESC}\\`,
  'end ' + osc('https://last.example', 'z'),
  osc('https://wrap.example/' + 'w'.repeat(30), 'a link that wraps onto the next line of the terminal') + ' after',
  osc('mailto:someone@example.com', 'mail') + ' ' + osc('not a url', 'bad')
]
// The links each provider finds from every line of a 40-column terminal.
function linkCase(line) {
  const cols = 40
  const t = new xterm.Headless(cols, 8, themeColors)
  t.feed(line)
  const rows = t.dump().lines.length
  const web = []
  const oscLinks = []
  for (let y = 1; y <= rows; y++) {
    web.push(xterm.computeLinks(t, y, strictUrlRegex))
    oscLinks.push(xterm.oscLinks(t, y))
  }
  return { line, cols, links: web, osc: oscLinks }
}
write('links.json', [...linkLines, ...oscLines].map(linkCase))
// ---------------------------------------------------------------------------
// registry.json
// ---------------------------------------------------------------------------

const scenarios = []
async function scenario(name, platform, steps) {
  const registry = await loadRegistry(platform)
  log = []
  const containers = new Map()
  const containerFor = (n) => {
    if (!containers.has(n)) containers.set(n, element())
    return containers.get(n)
  }
  const listeners = new Map()
  const tagTerminals = () => {
    for (const t of globalThis.__terminals) {
      if (!t.sessionId) t.sessionId = t.__session
    }
  }
  for (const step of steps) {
    log.push(['step', step.op])
    switch (step.op) {
      case 'container':
        containerFor(step.name).size = step.size
        break
      case 'attach': {
        const before = globalThis.__terminals.length
        const p = registry.attach(step.session, step.project ?? 'p1', containerFor(step.container), step.kind ?? 'claude')
        for (const t of globalThis.__terminals.slice(before)) t.sessionId = step.session
        p.then(
          () => log.push(['attached', step.id]),
          (err) => log.push(['attach-failed', step.id, err instanceof Error ? err.message : String(err)])
        )
        break
      }
      case 'detach':
        registry.detach(step.session, step.container ? containerFor(step.container) : undefined)
        break
      case 'dispose':
        registry.dispose(step.session)
        break
      case 'refit':
        registry.refit(step.session)
        break
      case 'focus':
        registry.focus(step.session)
        break
      case 'pty-data':
        handlers.data?.(step.session, step.data)
        break
      case 'pty-exit':
        handlers.exit?.(step.session)
        break
      case 'resolve-open':
        opens[step.n].resolve({ replay: step.replay })
        break
      case 'reject-open':
        opens[step.n].reject(new Error(step.message))
        break
      case 'type-in':
        registry.typeIn(step.session, step.text, { starting: !!step.starting })
        break
      case 'input':
        globalThis.__terminals.find((t) => t.sessionId === step.session && !t.disposed)?.emit(step.data)
        break
      case 'drop': {
        const term = globalThis.__terminals.filter((t) => t.sessionId === step.session).at(-1)
        const files = step.paths.map((p) => ({ path: p }))
        term.host.listeners.drop({ dataTransfer: { files, types: ['Files'] }, preventDefault() {} })
        break
      }
      case 'advance':
        await advance(step.ms)
        break
      case 'check':
        log.push(['state', step.session, registry.needsOpen(step.session), registry.hasOutput(step.session)])
        break
      case 'listen': {
        const off = registry.onOutput(step.session, () => log.push(['output', step.id]))
        listeners.set(step.id, off)
        break
      }
      case 'unlisten':
        listeners.get(step.id)?.()
        break
      default:
        throw new Error('unknown step ' + step.op)
    }
    tagTerminals()
    await settle()
  }
  scenarios.push({ name, platform, steps, log })
}

const card = { op: 'container', name: 'card', size: [100, 30] }
const modal = { op: 'container', name: 'modal', size: [120, 40] }
await scenario('first attach opens at the fitted size and replays', 'mac', [
  modal,
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'check', session: 's1' },
  { op: 'pty-data', session: 's1', data: 'dropped while opening' },
  { op: 'resolve-open', n: 0, replay: 'replayed\r\n' },
  { op: 'pty-data', session: 's1', data: 'live' },
  { op: 'pty-data', session: 's1', data: '' },
  { op: 'check', session: 's1' }
])
await scenario('exit while opening is written after the replay', 'mac', [
  modal,
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'pty-exit', session: 's1' },
  { op: 'resolve-open', n: 0, replay: 'bye' },
  { op: 'pty-data', session: 's1', data: 'ignored after exit' },
  { op: 'pty-exit', session: 's1' },
  { op: 'check', session: 's1' }
])
await scenario('exit lines per kind and reattach resets', 'mac', [
  card,
  { op: 'attach', id: 1, session: 'sh', container: 'card', kind: 'terminal' },
  { op: 'resolve-open', n: 0, replay: '$ ' },
  { op: 'pty-exit', session: 'sh' },
  { op: 'input', session: 'sh', data: 'typed after exit' },
  { op: 'check', session: 'sh' },
  { op: 'attach', id: 2, session: 'sh', container: 'card', kind: 'terminal' },
  { op: 'resolve-open', n: 1, replay: '$ again' },
  { op: 'check', session: 'sh' },
  { op: 'attach', id: 3, session: 'c1', container: 'card' },
  { op: 'resolve-open', n: 2, replay: '' },
  { op: 'check', session: 'c1' },
  { op: 'pty-exit', session: 'c1' }
])
await scenario('open failure leaves it resumable', 'mac', [
  modal,
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'reject-open', n: 0, message: 'Error: no claude here' },
  { op: 'check', session: 's1' },
  { op: 'input', session: 's1', data: 'x' },
  { op: 'attach', id: 2, session: 's1', container: 'modal' },
  { op: 'resolve-open', n: 1, replay: 'ok' },
  { op: 'check', session: 's1' }
])
await scenario('attaching while opening shares the open', 'mac', [
  modal,
  card,
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'input', session: 's1', data: 'typed ahead' },
  { op: 'attach', id: 2, session: 's1', container: 'card' },
  { op: 'resolve-open', n: 0, replay: 'r' },
  { op: 'check', session: 's1' }
])
await scenario('attach of a ready terminal moves, fits and resizes', 'mac', [
  modal,
  card,
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'resolve-open', n: 0, replay: 'r' },
  { op: 'attach', id: 2, session: 's1', container: 'card' },
  { op: 'detach', session: 's1', container: 'modal' },
  { op: 'detach', session: 's1', container: 'card' },
  { op: 'pty-data', session: 's1', data: 'parked output' },
  { op: 'container', name: 'card', size: [90, 20] },
  { op: 'refit', session: 's1' },
  { op: 'attach', id: 3, session: 's1', container: 'card' },
  { op: 'container', name: 'card', size: [80, 20] },
  { op: 'refit', session: 's1' },
  { op: 'detach', session: 's1' },
  { op: 'focus', session: 's1' }
])
await scenario('a container not laid out yet keeps the default size', 'mac', [
  { op: 'container', name: 'hidden', size: null },
  { op: 'attach', id: 1, session: 's1', container: 'hidden' },
  { op: 'resolve-open', n: 0, replay: '' }
])
await scenario('resize while opening is sent after the replay', 'mac', [
  modal,
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'container', name: 'modal', size: [110, 35] },
  { op: 'refit', session: 's1' },
  { op: 'resolve-open', n: 0, replay: 'r' }
])
await scenario('dispose during open does nothing more', 'mac', [
  modal,
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'type-in', session: 's1', text: 'queued' },
  { op: 'dispose', session: 's1' },
  { op: 'resolve-open', n: 0, replay: 'late' },
  { op: 'check', session: 's1' },
  { op: 'pty-data', session: 's1', data: 'nobody' },
  { op: 'pty-exit', session: 's1' }
])
await scenario('type in at once when ready', 'mac', [
  modal,
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'resolve-open', n: 0, replay: `${ESC}[?2004h> ` },
  { op: 'type-in', session: 's1', text: '  hello there \n' },
  { op: 'type-in', session: 's1', text: 'two\nlines' }
])
await scenario('typed words wait for the open, then go after the replay', 'mac', [
  modal,
  { op: 'type-in', session: 's1', text: 'first' },
  { op: 'type-in', session: 's1', text: 'second' },
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'resolve-open', n: 0, replay: `${ESC}[?2004h> ` }
])
await scenario('a starting session gets its words once it pastes bracketed', 'mac', [
  modal,
  { op: 'type-in', session: 's1', text: 'hi', starting: true },
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'resolve-open', n: 0, replay: 'starting up' },
  { op: 'advance', ms: 5000 },
  { op: 'pty-data', session: 's1', data: 'still loading' },
  { op: 'type-in', session: 's1', text: 'more' },
  { op: 'pty-data', session: 's1', data: `${ESC}[?2004h> ` },
  { op: 'advance', ms: 30000 }
])
await scenario('a starting session that never listens gets its words after 30 s', 'mac', [
  modal,
  { op: 'type-in', session: 's1', text: 'late words', starting: true },
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'resolve-open', n: 0, replay: 'no prompt' },
  { op: 'advance', ms: 29999 },
  { op: 'advance', ms: 1 }
])
await scenario('a starting session already listening gets them at once', 'mac', [
  modal,
  { op: 'type-in', session: 's1', text: 'now', starting: true },
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'resolve-open', n: 0, replay: `${ESC}[?2004h` }
])
await scenario('words wait when the session exits first', 'mac', [
  modal,
  { op: 'type-in', session: 's1', text: 'kept', starting: true },
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'resolve-open', n: 0, replay: 'x' },
  { op: 'pty-exit', session: 's1' },
  { op: 'advance', ms: 30000 },
  { op: 'attach', id: 2, session: 's1', container: 'modal' },
  { op: 'resolve-open', n: 1, replay: `${ESC}[?2004h` },
  { op: 'advance', ms: 30000 }
])
await scenario('output listeners', 'mac', [
  modal,
  { op: 'listen', session: 's1', id: 'before' },
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'listen', session: 's1', id: 'a' },
  { op: 'resolve-open', n: 0, replay: '' },
  { op: 'pty-data', session: 's1', data: 'x' },
  { op: 'unlisten', id: 'a' },
  { op: 'listen', session: 's1', id: 'b' },
  { op: 'pty-data', session: 's1', data: 'y' },
  { op: 'dispose', session: 's1' },
  { op: 'attach', id: 2, session: 's1', container: 'modal' },
  { op: 'resolve-open', n: 1, replay: 'z' }
])
await scenario('dropped files paste quoted paths', 'mac', [
  modal,
  { op: 'attach', id: 1, session: 's1', container: 'modal' },
  { op: 'resolve-open', n: 0, replay: `${ESC}[?2004h` },
  { op: 'drop', session: 's1', paths: ['/tmp/a.png', "/tmp/it's here.txt", ''] }
])
await scenario('unknown sessions are ignored', 'mac', [
  { op: 'pty-data', session: 'nope', data: 'x' },
  { op: 'pty-exit', session: 'nope' },
  { op: 'detach', session: 'nope' },
  { op: 'dispose', session: 'nope' },
  { op: 'focus', session: 'nope' },
  { op: 'check', session: 'nope' }
])
write('registry.json', scenarios)

fs.rmSync(work, { recursive: true, force: true })
