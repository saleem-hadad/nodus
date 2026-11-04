// Starts a packaged Nodus and drives it over the DevTools protocol, the way
// the Windows CI job checks a build no one has clicked through yet: the window
// opens, a terminal card's shell runs in the session host, the host answers on
// its pipe or socket, and each step is screenshotted in light and dark.
//
//   node scripts/smoke.mjs <path to Nodus.exe or …/Nodus.app/Contents/MacOS/Nodus> <folder for screenshots>
//
// It uses its own userData folder, so real boards are never touched.
import { spawn, spawnSync } from 'child_process'
import crypto from 'crypto'
import fs from 'fs'
import net from 'net'
import os from 'os'
import path from 'path'

const [exe, outDir = 'smoke'] = process.argv.slice(2)
if (!exe) throw new Error('usage: node scripts/smoke.mjs <app executable> [screenshot folder]')

const PORT = 9333
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'nodus-smoke-'))
const userData = path.join(root, 'userData')
const repo = path.join(root, 'demo')
fs.mkdirSync(repo, { recursive: true })
fs.mkdirSync(outDir, { recursive: true })

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const step = (text) => console.log(`\n▸ ${text}`)

async function until(what, fn, timeoutMs = 60_000) {
  const end = Date.now() + timeoutMs
  for (;;) {
    try {
      const value = await fn()
      if (value) return value
    } catch {
      // not yet
    }
    if (Date.now() > end) throw new Error(`Timed out waiting for ${what}`)
    await sleep(250)
  }
}

step(`Starting ${exe}`)
const app = spawn(exe, [`--remote-debugging-port=${PORT}`], {
  env: { ...process.env, NODUS_USER_DATA: userData },
  stdio: ['ignore', 'pipe', 'pipe']
})
const appLog = fs.createWriteStream(path.join(outDir, 'app.log'))
app.stdout.pipe(appLog)
app.stderr.pipe(appLog)
let exited = null
app.on('exit', (code) => (exited = code))

let ws
let nextId = 1
const pending = new Map()

function send(method, params = {}) {
  const id = nextId++
  ws.send(JSON.stringify({ id, method, params }))
  return new Promise((resolve, reject) => pending.set(id, { resolve, reject }))
}

/** Evaluates an expression in the window, awaiting a promise, and returns its value. */
async function evaluate(expression) {
  const { result, exceptionDetails } = await send('Runtime.evaluate', {
    expression,
    awaitPromise: true,
    returnByValue: true
  })
  if (exceptionDetails) throw new Error(`${expression}\n${exceptionDetails.exception?.description ?? exceptionDetails.text}`)
  return result.value
}

async function screenshot(name) {
  await sleep(800)
  const { data } = await send('Page.captureScreenshot', { format: 'png' })
  fs.writeFileSync(path.join(outDir, `${name}.png`), Buffer.from(data, 'base64'))
  console.log(`  saved ${name}.png`)
}

/** Double-clicks the middle of the first element matching `selector`. */
async function doubleClick(selector) {
  const box = await evaluate(`(() => {
    const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect()
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 }
  })()`)
  for (const clickCount of [1, 2]) {
    await send('Input.dispatchMouseEvent', { type: 'mousePressed', ...box, button: 'left', clickCount })
    await send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...box, button: 'left', clickCount })
  }
}

/** Runs a Windows PowerShell script, and returns its exit code. Encoded, so no quote in it needs escaping. */
const powershell = (script) =>
  spawnSync(
    'powershell.exe',
    ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(script, 'utf16le').toString('base64')],
    { stdio: 'inherit' }
  ).status

/**
 * Presses Ctrl (and Shift) and a letter through Windows, so it reaches the
 * app as a user's would, menu accelerators included, which keys sent over
 * DevTools skip. SendKeys' ^ doesn't reach Chromium as Ctrl, so this holds
 * the keys down itself. False if the window couldn't come to the front.
 */
function pressCtrl(letter, { shift = false } = {}) {
  const vk = letter.toUpperCase().charCodeAt(0)
  const keys = [0x11, ...(shift ? [0x10] : []), vk]
  const down = keys.map((k) => `[Keys]::keybd_event(${k}, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 40`)
  const up = keys.reverse().map((k) => `[Keys]::keybd_event(${k}, 0, 2, [UIntPtr]::Zero); Start-Sleep -Milliseconds 40`)
  const status = powershell(
    `Add-Type -Namespace Smoke -Name Keys -MemberDefinition '[DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);'; ` +
      `$s = New-Object -ComObject WScript.Shell; if (-not $s.AppActivate(${app.pid})) { exit 3 }; Start-Sleep -Milliseconds 400; ` +
      [...down, ...up].join('; ').replaceAll('[Keys]', '[Smoke.Keys]')
  )
  return status === 0
}

/** The whole screen, with the window's own buttons, which the page's screenshots leave out. */
function screenCapture(name) {
  const file = path.resolve(outDir, `${name}.png`).replace(/'/g, "''")
  powershell(
    'Add-Type -AssemblyName System.Windows.Forms, System.Drawing; ' +
      '$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; ' +
      '$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height; ' +
      '[System.Drawing.Graphics]::FromImage($bmp).CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size); ' +
      `$bmp.Save('${file}')`
  )
  console.log(`  saved ${name}.png`)
}

/** Connects to the session host the way the app does, and returns its welcome. */
function hostWelcome() {
  const dir = path.join(userData, 'host')
  const hash = crypto.createHash('sha1').update(dir).digest('hex').slice(0, 12)
  let socket = path.join(dir, 'host.sock')
  if (process.platform === 'win32') socket = `\\\\.\\pipe\\nodus-${hash}`
  else if (Buffer.byteLength(socket) > 100) socket = path.join(os.tmpdir(), `nodus-${hash}.sock`)
  return new Promise((resolve, reject) => {
    const conn = net.createConnection(socket)
    let text = ''
    conn.on('error', reject)
    conn.on('data', (chunk) => {
      text += chunk
      const nl = text.indexOf('\n')
      if (nl < 0) return
      conn.destroy()
      resolve({ socket, welcome: JSON.parse(text.slice(0, nl)) })
    })
    // Protocol 0 is never the app's, so the host doesn't count this as an app.
    conn.write(JSON.stringify({ type: 'hello', protocol: 0 }) + '\n')
  })
}

let failed = false
try {
  step('Waiting for the window')
  const target = await until('the window', async () => {
    if (exited !== null) throw new Error(`Nodus exited with ${exited}`)
    const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json()
    return list.find((t) => t.type === 'page' && t.url.includes('index.html'))
  })
  ws = new WebSocket(target.webSocketDebuggerUrl)
  await new Promise((resolve, reject) => {
    ws.onopen = resolve
    ws.onerror = reject
  })
  ws.onmessage = (event) => {
    const msg = JSON.parse(event.data)
    const waiter = pending.get(msg.id)
    if (!waiter) return
    pending.delete(msg.id)
    if (msg.error) waiter.reject(new Error(msg.error.message))
    else waiter.resolve(msg.result)
  }
  await until('the page', () => evaluate(`document.readyState === 'complete' && !!window.api`))

  const info = await evaluate('window.api.appInfo()')
  console.log('  app info', info)
  const platform = await evaluate('document.documentElement.dataset.platform')
  const menuButton = await evaluate(`!!document.querySelector('.titlebar-menu')`)
  console.log(`  data-platform ${platform}, menu button ${menuButton ? 'shown' : 'not shown'}`)
  await screenshot('1-projects-light')

  step('Adding a project and opening its board')
  const project = await evaluate(`window.api.projects.add(${JSON.stringify(repo)})`)
  await evaluate('location.reload()').catch(() => {})
  await until('the project on the grid', () => evaluate(`!!document.querySelector('[data-project-id]')`))
  await doubleClick('[data-project-id]')
  await until('the board', () => evaluate(`!!document.querySelector('.react-flow')`))

  step('Starting a terminal card')
  await evaluate(`(() => {
    window.__out = ''
    window.api.on.ptyData((_id, data) => { window.__out += data })
  })()`)
  const card = await evaluate(
    `window.api.sessions.create(${JSON.stringify(project.id)}, { x: 80, y: 80, kind: 'terminal', expanded: true })`
  )
  await evaluate(`window.api.sessions.open(${JSON.stringify(project.id)}, ${JSON.stringify(card.sessionId)}, 120, 36)`)
  // Only the shell's output has the sum; the typed line shows the sum's parts.
  const sum = process.platform === 'win32' ? 'echo ("nodus-" + (6 * 7))' : 'echo nodus-$((6 * 7))'
  await until('the shell prompt', () => evaluate(`window.__out.length > 0`), 30_000)
  await evaluate(`window.api.sessions.write(${JSON.stringify(card.sessionId)}, ${JSON.stringify(sum + '\r')})`)
  await until('the shell to answer', () => evaluate(`window.__out.includes('nodus-42')`), 30_000)
  console.log('  the shell answered')

  // The board shows a card made outside it once it opens again.
  await evaluate('location.reload()').catch(() => {})
  await until('the project on the grid', () => evaluate(`!!document.querySelector('[data-project-id]')`))
  await doubleClick('[data-project-id]')
  await until('the terminal card', () => evaluate(`!!document.querySelector('.xterm')`), 30_000)
  await screenshot('2-board-light')

  step('Asking the session host for its sessions')
  const { socket, welcome } = await hostWelcome()
  console.log(`  ${socket}: protocol ${welcome.protocol}, ${welcome.sessions.length} session(s)`)
  if (!welcome.sessions.some((s) => s.sessionId === card.sessionId)) throw new Error('The host doesn’t have the terminal')

  step('Dark mode')
  await evaluate(`window.api.settings.update({ theme: 'dark' })`)
  await screenshot('3-board-dark')

  if (process.platform === 'win32') {
    step('The window, and keys pressed through Windows')
    await evaluate(`window.api.settings.update({ theme: 'light' })`)
    const onBoard = () => evaluate(`!!document.querySelector('.react-flow')`)
    const reopen = async () => {
      await doubleClick('[data-project-id]')
      await until('the terminal card', () => evaluate(`!!document.querySelector('.xterm')`), 30_000)
    }
    const focusTerminal = () => evaluate(`document.querySelector('.xterm-helper-textarea').focus()`)
    await focusTerminal()
    // Ctrl+W on the board closes it, as ⌘W does on a Mac. First, as that shows the keys get through.
    await evaluate(`document.activeElement?.blur()`)
    if (!pressCtrl('w')) {
      console.log('  skipped: the window couldn’t come to the front')
    } else {
      await until('Ctrl+W on the board to close it', async () => !(await onBoard()), 10_000)
      console.log('  Ctrl+W on the board went back to the projects')
      await reopen()

      // In a terminal, Ctrl+W is the terminal's own key, so the board stays.
      await focusTerminal()
      pressCtrl('w')
      await sleep(1500)
      screenCapture('4-screen')
      if (!(await onBoard())) throw new Error('Ctrl+W in a terminal closed the board')
      console.log('  Ctrl+W in the terminal stayed with the terminal')

      // Ctrl+Shift+W is the app's.
      await focusTerminal()
      pressCtrl('w', { shift: true })
      await until('Ctrl+Shift+W in the terminal to close the board', async () => !(await onBoard()), 10_000)
      console.log('  Ctrl+Shift+W in the terminal went back to the projects')
    }
  }

  console.log('\n✓ Smoke test passed')
} catch (err) {
  failed = true
  console.error('\n✗ Smoke test failed:', err)
  await screenshot('failure').catch(() => {})
} finally {
  // Stops the host and its sessions, then the app.
  try {
    const { socket } = await hostWelcome()
    const conn = net.createConnection(socket)
    conn.write(JSON.stringify({ type: 'hello', protocol: 0 }) + '\n' + JSON.stringify({ type: 'shutdown' }) + '\n')
    await sleep(1000)
    conn.destroy()
  } catch {
    // no host
  }
  app.kill()
  await sleep(1000)
  const hostLog = path.join(userData, 'host', 'host.log')
  if (fs.existsSync(hostLog)) fs.copyFileSync(hostLog, path.join(outDir, 'host.log'))
  process.exit(failed ? 1 : 0)
}
