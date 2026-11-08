// Launches the Electron app on a seeded world and drives it for the scenes:
// a private userData and HOME, stand-ins for claude and the login shell, a
// fixed 1440×900 window at 2x, a frozen clock in the renderer, and session
// statuses driven through the session host's hook endpoint.
import crypto from 'node:crypto'
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import http from 'node:http'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { _electron as electron } from 'playwright-core'
import { NOW, USAGE, projectId, sessionId } from '../seed/world.mjs'
import { changedBox, readPng } from './pixels.mjs'

export const WIDTH = 1440
export const HEIGHT = 900
export const SCALE = 2

/** Electron's binary in an app checkout, as `require('electron')` would find it. */
export function electronBinary(appDir) {
  const dir = path.join(appDir, 'node_modules', 'electron')
  const rel = fs.readFileSync(path.join(dir, 'path.txt'), 'utf8').trim()
  return path.join(dir, 'dist', rel)
}

/** What every page sees before its own scripts: a clock frozen at NOW. */
const INIT_SCRIPT = `(() => {
  const FIXED = ${NOW};
  const RealDate = Date;
  function FrozenDate(...args) {
    if (!new.target) return new RealDate(FIXED).toString();
    return args.length ? new RealDate(...args) : new RealDate(FIXED);
  }
  FrozenDate.prototype = RealDate.prototype;
  FrozenDate.now = () => FIXED;
  FrozenDate.parse = RealDate.parse;
  FrozenDate.UTC = RealDate.UTC;
  Object.defineProperty(globalThis, 'Date', { value: FrozenDate, writable: true, configurable: true });
})();`

/** The environment the app starts with: nothing from the user's own, except what macOS needs. */
function appEnv(world) {
  const bin = path.join(world.work, 'bin')
  return {
    HOME: world.home,
    USER: os.userInfo().username,
    LOGNAME: os.userInfo().username,
    TMPDIR: os.tmpdir(),
    PATH: `${bin}:/usr/bin:/bin`,
    SHELL: path.join(bin, 'fakeshell'),
    LANG: 'en_US.UTF-8',
    TZ: 'UTC',
    NODUS_USER_DATA: world.userData,
    PARITY_RESPONSES: path.join(world.work, 'responses.json'),
    BASH_SILENCE_DEPRECATION_WARNING: '1'
  }
}

/** The session host's socket for a userData folder (hostPaths in src/main/hostProtocol.ts). */
export function hostSocket(userData) {
  const dir = path.join(userData, 'host')
  const socket = path.join(dir, 'host.sock')
  if (Buffer.byteLength(socket) <= 100) return socket
  const hash = crypto.createHash('sha1').update(dir).digest('hex').slice(0, 12)
  return path.join(os.tmpdir(), `nodus-${hash}.sock`)
}

export class App {
  constructor({ appDir, buildDir, world, theme, chromiumSandbox = true }) {
    Object.assign(this, { appDir, buildDir, world, theme, chromiumSandbox })
    this.hookEndpoint = null
  }

  async launch() {
    const args = [path.join(this.buildDir, 'main', 'index.js'), `--force-device-scale-factor=${SCALE}`, '--lang=en-US']
    // Chromium's own sandbox can't start inside another one (a sandboxed agent's shell, say).
    if (!this.chromiumSandbox) args.push('--no-sandbox')
    this.electron = await electron.launch({
      executablePath: electronBinary(this.appDir),
      args,
      env: appEnv(this.world),
      colorScheme: this.theme,
      locale: 'en-US',
      timezoneId: 'UTC',
      timeout: 60_000
    })
    await this.electron.context().addInitScript(INIT_SCRIPT)
    this.page = await this.electron.firstWindow()
    await this.setupWindow()
    // macOS keeps a window inside the screen's work area, which may be shorter
    // than 900 points, so the page gets its size from emulation instead.
    await this.emulateSize(this.page, WIDTH, HEIGHT)
    // The first page loaded before the init script was in; load it again with it.
    await this.page.reload()
    await this.page.waitForSelector('.app')
    await this.page.evaluate(() => document.fonts.ready)
    return this
  }

  /** Keeps the user's real pointer and keyboard out of the window, fixes its size, and silences notifications. */
  async setupWindow() {
    await this.electron.evaluate(async ({ BrowserWindow, Notification }, { width, height }) => {
      const win = BrowserWindow.getAllWindows().find((w) => !w.webContents.getURL().includes('#capture'))
      if (!win.isVisible()) await new Promise((resolve) => win.once('show', resolve))
      win.setIgnoreMouseEvents(true)
      win.setFocusable(false)
      win.blur()
      win.setContentSize(width, height)
      win.webContents.setBackgroundThrottling(false)
      Notification.prototype.show = function () {}
    }, { width: WIDTH, height: HEIGHT })
  }

  /** Gives a page a fixed CSS size at SCALE device pixels per CSS pixel. */
  async emulateSize(page, width, height) {
    const cdp = await this.electron.context().newCDPSession(page)
    await cdp.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: SCALE, mobile: false })
    page.parityCdp = cdp
  }

  /** Sets up a window that isn't the board's (the capture panel) the same way. */
  async quietWindow(match) {
    await this.electron.evaluate(({ BrowserWindow }, match) => {
      for (const win of BrowserWindow.getAllWindows()) {
        if (!win.webContents.getURL().includes(match)) continue
        win.setIgnoreMouseEvents(true)
        win.webContents.setBackgroundThrottling(false)
      }
    }, match)
  }

  async close() {
    try {
      await this.electron?.close()
    } catch {
      // Already gone.
    }
    // The session host outlives the app, with the stand-ins it runs.
    try {
      execFileSync('pkill', ['-f', path.join(this.buildDir, 'main', 'host.js')], { stdio: 'ignore' })
    } catch {}
    try {
      execFileSync('pkill', ['-f', path.join(this.world.work, 'bin')], { stdio: 'ignore' })
    } catch {}
  }

  // -------------------------------------------------------------------------
  // The session host

  /** The host's hook port and token, read from its welcome. The token stays in memory. */
  async endpoint() {
    if (this.hookEndpoint) return this.hookEndpoint
    const socketPath = hostSocket(this.world.userData)
    for (let i = 0; i < 100 && !fs.existsSync(socketPath); i++) await sleep(100)
    this.hookEndpoint = await new Promise((resolve, reject) => {
      const socket = net.connect(socketPath)
      let buf = ''
      socket.on('data', (d) => {
        buf += d.toString('utf8')
        const nl = buf.indexOf('\n')
        if (nl < 0) return
        const msg = JSON.parse(buf.slice(0, nl))
        socket.destroy()
        resolve({ port: msg.hookPort, token: msg.hookToken })
      })
      socket.on('error', reject)
      socket.write(JSON.stringify({ type: 'hello', protocol: 0 }) + '\n')
    })
    return this.hookEndpoint
  }

  /** Fires a Claude Code hook for a session, as its process would. */
  async hook(sessionKey, event, payload = {}) {
    const { port, token } = await this.endpoint()
    const body = JSON.stringify(payload)
    await new Promise((resolve, reject) => {
      const req = http.request(
        {
          host: '127.0.0.1',
          port,
          method: 'POST',
          path: `/hook?evt=${event}&sid=${sessionId(sessionKey)}`,
          headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(body) }
        },
        (res) => {
          res.resume()
          res.on('end', () =>
            res.statusCode < 300 ? resolve() : reject(new Error(`Hook ${event} for ${sessionKey}: HTTP ${res.statusCode}`))
          )
        }
      )
      req.on('error', reject)
      req.end(body)
    })
  }

  /** Starts sessions' processes (the stand-ins), as opening their cards would. */
  async liven(projectKey, sessionKeys, cols = 120, rows = 36) {
    await this.page.evaluate(
      async ({ pid, sids, cols, rows }) => {
        for (const sid of sids) await window.api.sessions.open(pid, sid, cols, rows)
      },
      { pid: projectId(projectKey), sids: sessionKeys.map(sessionId), cols, rows }
    )
  }

  /** Puts sessions in a status: working, waiting or done (live, through hooks). */
  async status(sessionKey, status, agentId) {
    const payload = agentId ? { agent_id: agentId } : {}
    if (status === 'working') await this.hook(sessionKey, agentId ? 'SubagentStart' : 'UserPromptSubmit', payload)
    else if (status === 'waiting') await this.hook(sessionKey, 'PermissionRequest', payload)
    else await this.hook(sessionKey, agentId ? 'SubagentStop' : 'Stop', payload)
  }

  /** Reports a session's model, context and cost through its status line, as Claude would. */
  async usage(sessionKey) {
    await this.hook(sessionKey, 'StatusLine', USAGE[sessionKey])
  }

  // -------------------------------------------------------------------------
  // Navigation

  /** Opens a project's board from the grid, with a double-click, as a user does. */
  async openProject(projectKey) {
    const item = this.page.locator(`.pv-item[data-project-id="${projectId(projectKey)}"]`)
    await item.dblclick()
    await this.page.waitForSelector('.react-flow__viewport')
    await this.page.waitForFunction(() => !document.querySelector('.board-loading'))
  }

  /** Runs a menu item from the native application menu, by its path of labels. */
  async menu(...labels) {
    await this.electron.evaluate(({ Menu, BrowserWindow }, labels) => {
      let items = Menu.getApplicationMenu().items
      let item
      for (const label of labels) {
        item = items.find((i) => i.label === label)
        if (!item) throw new Error(`No menu item ${labels.join(' › ')}`)
        items = item.submenu?.items ?? []
      }
      const win = BrowserWindow.getAllWindows().find((w) => !w.webContents.getURL().includes('#capture'))
      item.click(undefined, win, win.webContents)
    }, labels)
  }

  /** Sends the board window a menu command (ev:menu), as the menu's accelerators do. */
  async menuCommand(command) {
    await this.electron.evaluate(({ BrowserWindow }, command) => {
      const win = BrowserWindow.getAllWindows().find((w) => !w.webContents.getURL().includes('#capture'))
      win.webContents.send('ev:menu', command)
    }, command)
  }
}

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

// ---------------------------------------------------------------------------
// Screenshots

const SHOT = { caret: 'hide', scale: 'device' }

/**
 * Holds every infinite animation (status dots breathing and pulsing, spinners)
 * at its first keyframe, so each shot catches the same phase. Elements drawn
 * since the last call start theirs running, so it runs before every shot.
 */
export async function freezeAnimations(page) {
  await page.evaluate(async () => {
    for (const a of document.getAnimations()) {
      if (a.effect?.getComputedTiming().iterations !== Infinity) continue
      a.pause()
      a.currentTime = 0
    }
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))
  })
}

/**
 * Waits for the page to stop changing, then takes the shot: fonts loaded,
 * finite animations run out, infinite ones held at their first keyframe, and
 * two shots in a row the same.
 */
export async function capture(page, file, { clip, settleMs = 300, tries = 12 } = {}) {
  await page.evaluate(() => document.fonts.ready)
  await page
    .waitForFunction(
      () =>
        document
          .getAnimations()
          .every((a) => a.playState !== 'running' || a.effect?.getComputedTiming().iterations === Infinity),
      null,
      { timeout: 5000, polling: 50 }
    )
    .catch(() => {})
  await sleep(settleMs)
  await freezeAnimations(page)
  let last = await page.screenshot({ ...SHOT, clip })
  let stable = false
  let changing = null
  for (let i = 0; i < tries; i++) {
    await sleep(150)
    await freezeAnimations(page)
    const next = await page.screenshot({ ...SHOT, clip })
    if (next.equals(last)) {
      stable = true
      break
    }
    changing = changedBox(readPng(last), readPng(next))
    last = next
  }
  fs.mkdirSync(path.dirname(file), { recursive: true })
  fs.writeFileSync(file, last)
  return stable ? { stable } : { stable, changing }
}
