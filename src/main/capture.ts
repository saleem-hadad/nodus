// Floating capture: double-tap Shift in any app and a panel floats up over it.
// What's typed there becomes a session, a note or a view on a project's
// board, or the next prompt of a session already on it; Claude guesses which
// (captureGuess.ts) and the user can change it.
//
// The panel is a window of its own, an NSPanel on a Mac, so it takes the
// keyboard without bringing the app or its board window forward, and floats
// over full-screen apps on every Space. Its page is the renderer's, at
// #capture. The double tap is heard through a global key hook (uiohook-napi),
// which on a Mac needs Accessibility permission.
import { app, BrowserWindow, dialog, ipcMain, screen, shell, systemPreferences } from 'electron'
import crypto from 'crypto'
import fs from 'fs'
import path from 'path'
import { pathToFileURL } from 'url'
import { IPC } from '@shared/api'
import { isCaptureKind, titleFrom, withScreen } from '@shared/capture'
import { VIEW_H, VIEW_W, cardSize, dockRect, findFreeSpot, type Point, type Size } from '@shared/geometry'
import type {
  BoardNote,
  CaptureAttachment,
  CaptureContext,
  CaptureReceipt,
  CaptureRecent,
  CaptureSession,
  CaptureShortcut,
  CaptureSubmission,
  CaptureTargets,
  CreateSessionOptions,
  OpenCardRequest,
  SessionCard
} from '@shared/types'
import { cancelGuess, guessCapture } from './captureGuess'
import { readSession, transcriptPath } from './discovery'
import { DoubleTap } from './doubleTap'
import { getStatus } from './hooks'
import { fitNote } from './notes'
import { isLive } from './pty'
import {
  browserTab,
  frontApp,
  isBrowser,
  pruneShots,
  readClipboard,
  takeClipboard,
  takeScreenshot
} from './screenContext'
import { getBoard, getProject, getState, save, type BoardState, type CaptureRecord } from './store'
import * as views from './views'

const isMac = process.platform === 'darwin'

/**
 * The panel's window: the card's width and the room its menu and recents
 * need, with a see-through margin its shadow fits in (.cap-stage).
 */
const PANEL_W = 700
const PANEL_H = 640
/** From the window's top to the card's. */
const CARD_TOP = 56
/** How far down the screen the card's top sits. */
const PANEL_TOP = 0.2
/** How many captures are remembered for the recents. */
const MAX_RECORDS = 20
const MAX_TEXT_CHARS = 20_000

export interface CaptureHandlers {
  /** The board window, if open. */
  mainWindow(): BrowserWindow | null
  createSession(projectId: string, opts: CreateSessionOptions): Promise<SessionCard>
  sendToSession(projectId: string, sessionId: string, text: string): Promise<void>
  sessionTitle(projectId: string, sessionId: string): string
  boardChanged(projectId: string): void
  openCard(request: OpenCardRequest): void
  /** Brings up the board window's Settings. */
  openSettings(): void
}

let h: CaptureHandlers | null = null
let panel: BrowserWindow | null = null
/** A screenshot or a file dialog is up: losing focus to it doesn't hide the panel. */
let busy = false
/** The receipt is up, without the keyboard, which went back to the app the user was in. */
let resting = false
/** When the panel last came up. */
let shownAt = 0
/** A blur this soon after the panel came up is left over from before. */
const BLUR_GRACE_MS = 250
/** The board open last in the window, for where a capture goes before Claude has guessed. */
let lastBoard: string | null = null
/** What the panel opened onto, for the guesses. */
let openedOn: { app?: string; targets: CaptureTargets } | null = null
/** What the panel's page was last told it's up over, for a page that loads after being told. */
let shownContext: CaptureContext | null = null

// ---------------------------------------------------------------------------
// The panel

const rendererUrl = () =>
  `${process.env.ELECTRON_RENDERER_URL ?? pathToFileURL(path.join(__dirname, '../renderer/index.html')).href}#capture`

function createPanel(): BrowserWindow {
  const win = new BrowserWindow({
    width: PANEL_W,
    height: PANEL_H,
    show: false,
    frame: false,
    transparent: true,
    backgroundColor: '#00000000',
    hasShadow: false,
    resizable: false,
    movable: false,
    minimizable: false,
    maximizable: false,
    fullscreenable: false,
    skipTaskbar: true,
    alwaysOnTop: true,
    // The receipt's link works on the first click, while the panel doesn't have the keyboard.
    acceptFirstMouse: true,
    // An NSPanel: takes the keyboard without activating the app.
    ...(isMac ? { type: 'panel' as const } : {}),
    title: 'Nodus Capture',
    webPreferences: {
      preload: path.join(__dirname, '../preload/index.js'),
      sandbox: true,
      contextIsolation: true,
      nodeIntegration: false
    }
  })
  // Over full-screen apps, on every Space, without turning Nodus into an app with no Dock icon.
  if (isMac) win.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true, skipTransformProcessType: true })
  win.setAlwaysOnTop(true, 'pop-up-menu')

  // The page gets window.api, so it never leaves the app's own page.
  win.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:/.test(url)) void shell.openExternal(url)
    return { action: 'deny' }
  })
  win.webContents.on('will-navigate', (event) => event.preventDefault())

  // Clicking away puts it away. A blur can arrive late, from hiding it just before it was shown again,
  // so only one that leaves it unfocused a moment later counts.
  win.on('blur', () => {
    if (busy || resting || Date.now() - shownAt < BLUR_GRACE_MS) return
    setTimeout(() => {
      if (!busy && !win.isDestroyed() && !win.isFocused()) hidePanel()
    }, 50)
  })
  win.on('closed', () => {
    if (panel === win) panel = null
  })

  void win.loadURL(rendererUrl())
  return win
}

function getPanel(): BrowserWindow {
  if (!panel || panel.isDestroyed()) panel = createPanel()
  return panel
}

/**
 * Tells the panel's page what it's up over. A page still loading asks for it
 * when it's ready (captureReady), since its code loads after the page does.
 */
function tellPanel(win: BrowserWindow, context: Partial<CaptureContext>, shown: boolean) {
  shownContext = shown ? (context as CaptureContext) : { ...shownContext!, ...context }
  if (win.webContents.isLoading()) return
  win.webContents.send(shown ? IPC.evCaptureShown : IPC.evCaptureContext, context)
}

/** Centred on the screen the pointer is on, a fifth of the way down. */
function place(win: BrowserWindow) {
  const { workArea } = screen.getDisplayNearestPoint(screen.getCursorScreenPoint())
  win.setBounds({
    x: Math.round(workArea.x + (workArea.width - PANEL_W) / 2),
    y: Math.round(workArea.y + workArea.height * PANEL_TOP - CARD_TOP),
    width: PANEL_W,
    height: PANEL_H
  })
}

export const isPanelOpen = () => Boolean(panel && !panel.isDestroyed() && panel.isVisible())

/** Floats the panel up over whatever app is in front. */
export async function openPanel() {
  // Read before the panel shows: Nodus may be the app in front, but the panel never is.
  const own = BrowserWindow.getFocusedWindow() !== null
  const front = own ? null : await frontApp()
  const win = getPanel()
  place(win)
  openedOn = { app: front?.name, targets: targets() }
  tellPanel(win, { app: front?.name, shortcut: shortcutState() }, true)
  // Clicks go through until the pointer is over the card; its page says when (setPassThrough).
  win.setIgnoreMouseEvents(true, { forward: true })
  shownAt = Date.now()
  resting = false
  win.show()
  win.focus()
  // What takes longer arrives after the panel is up.
  const clipboard = await readClipboard()
  if (clipboard && isPanelOpen()) tellPanel(win, { clipboard }, false)
  if (front && isBrowser(front.bundleId)) {
    const tab = await browserTab(front)
    if (tab && isPanelOpen()) tellPanel(win, { tab }, false)
  }
}

export function hidePanel() {
  cancelGuess()
  resting = false
  if (panel && !panel.isDestroyed() && panel.isVisible()) panel.hide()
}

/** The double tap, and the menu's Capture item: open it, or put it away. */
export function togglePanel() {
  if (isPanelOpen() && panel!.isFocused()) hidePanel()
  else void openPanel()
}

/**
 * Gives the keyboard back to the app the user was in, keeping the panel up
 * for its receipt until its page hides it.
 */
function rest() {
  if (!panel || panel.isDestroyed() || !panel.isVisible()) return
  resting = true
  panel.hide()
  panel.showInactive()
}

/** Hides the panel while the user does something outside it, then brings it back. */
async function aside<T>(work: (win: BrowserWindow) => Promise<T>): Promise<T> {
  const win = getPanel()
  busy = true
  try {
    return await work(win)
  } finally {
    busy = false
    if (!win.isDestroyed()) {
      win.show()
      win.focus()
    }
  }
}

// ---------------------------------------------------------------------------
// The double tap

let hook: { stop(): void } | null = null
let hookFailed = false
let permissionTimer: NodeJS.Timeout | null = null

const trusted = () => !isMac || systemPreferences.isTrustedAccessibilityClient(false)

export function shortcutState(): CaptureShortcut {
  if (!getState().settings.capture) return 'off'
  if (hookFailed) return 'unavailable'
  if (!trusted()) return 'needs-permission'
  return 'ready'
}

function startHook() {
  if (hook || hookFailed) return
  try {
    // Loaded here rather than at the top, so a native addon that can't load only costs the shortcut.
    const { uIOhook } = require('uiohook-napi') as typeof import('uiohook-napi')
    // Timed here: libuiohook's own event times are raw clock ticks on a Mac, not milliseconds.
    const tap = new DoubleTap(() => togglePanel())
    uIOhook.on('keydown', (e) => tap.keyDown(e.keycode, performance.now()))
    uIOhook.on('keyup', (e) => tap.keyUp(e.keycode, performance.now()))
    uIOhook.on('mousedown', () => tap.other())
    uIOhook.start()
    hook = {
      stop: () => {
        uIOhook.removeAllListeners()
        uIOhook.stop()
      }
    }
    // The first double tap shouldn't wait for the page to load.
    setTimeout(() => getPanel(), 3000)
  } catch (err) {
    hookFailed = true
    console.error('[capture] the key hook failed to start', err)
  }
}

function stopHook() {
  try {
    hook?.stop()
  } catch (err) {
    console.error('[capture] the key hook failed to stop', err)
  }
  hook = null
}

/** Starts or stops listening for the double tap, to match the setting and the permission. */
export function syncShortcut() {
  // Settings are applied once before capture is registered.
  if (!h) return
  if (permissionTimer) clearInterval(permissionTimer)
  permissionTimer = null
  if (!getState().settings.capture) return stopHook()
  if (trusted()) return startHook()
  // Until macOS trusts Nodus, check now and then: the user may be in System Settings allowing it.
  permissionTimer = setInterval(() => {
    if (!trusted()) return
    clearInterval(permissionTimer!)
    permissionTimer = null
    startHook()
  }, 3000)
}

export function stopCapture() {
  if (permissionTimer) clearInterval(permissionTimer)
  stopHook()
}

/** Asks macOS for Accessibility permission, and opens the pane where it's given. */
function askPermission(): CaptureShortcut {
  if (isMac && !trusted()) {
    systemPreferences.isTrustedAccessibilityClient(true)
    void shell.openExternal('x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility')
    syncShortcut()
  }
  return shortcutState()
}

// ---------------------------------------------------------------------------
// Projects, sessions and recents

/** Remembers the board the window opened, as where captures go before Claude has guessed. */
export function boardOpened(projectId: string) {
  lastBoard = projectId
}

function sessionInfo(projectId: string, sessionId: string): CaptureSession | null {
  const project = getProject(projectId)
  const node = getBoard(projectId).nodes[sessionId]
  if (!node || node.archived || node.kind === 'terminal' || node.runOf || node.viewOf) return null
  const meta = readSession(sessionId, transcriptPath(project.repoPath, sessionId))?.meta
  // Until Claude names a session started here, it goes by what was captured.
  const captured = getState().captures?.find((c) => c.kind === 'session' && c.sessionId === sessionId)?.title
  return {
    projectId,
    sessionId,
    title: node.title || meta?.title || captured || 'Untitled session',
    status: getStatus(sessionId),
    live: isLive(sessionId),
    updatedAt: meta?.updatedAt ?? node.createdAt ?? 0
  }
}

function recentOf(record: CaptureRecord): CaptureRecent | null {
  const board = getState().boards[record.projectId]
  if (!board || !getState().projects.some((p) => p.id === record.projectId)) return null
  const base: CaptureRecent = {
    id: record.id,
    kind: record.kind,
    projectId: record.projectId,
    title: record.title,
    createdAt: record.createdAt
  }
  switch (record.kind) {
    case 'session':
    case 'into': {
      const session = record.sessionId ? sessionInfo(record.projectId, record.sessionId) : null
      if (!session) return null
      // A session goes by Claude's title for it once it has one (sessionInfo).
      return { ...base, title: session.title, sessionId: session.sessionId, status: session.status }
    }
    case 'note': {
      const note = record.noteId ? board.notes?.[record.noteId] : undefined
      return note ? { ...base, noteId: note.id } : null
    }
    case 'view': {
      const view = record.viewId ? board.views?.[record.viewId] : undefined
      return view ? { ...base, title: view.title === 'New view' ? record.title : view.title, viewId: view.id } : null
    }
  }
}

function defaultProject(projects: { id: string }[]): string | null {
  const ids = new Set(projects.map((p) => p.id))
  const lastCapture = getState().captures?.find((c) => ids.has(c.projectId))
  const opened = getState()
    .projects.filter((p) => ids.has(p.id))
    .sort((a, b) => (b.openedAt ?? 0) - (a.openedAt ?? 0))[0]
  if (lastCapture && (!opened?.openedAt || lastCapture.createdAt > opened.openedAt)) return lastCapture.projectId
  if (lastBoard && ids.has(lastBoard)) return lastBoard
  return opened?.id ?? null
}

export function targets(): CaptureTargets {
  const projects = getState().projects.filter((p) => !p.archived)
  const sessions: CaptureSession[] = []
  for (const p of projects) {
    for (const node of Object.values(getBoard(p.id).nodes)) {
      const info = sessionInfo(p.id, node.sessionId)
      if (info) sessions.push(info)
    }
  }
  sessions.sort((a, b) => b.updatedAt - a.updatedAt)
  const recents = (getState().captures ?? []).map(recentOf).filter((r): r is CaptureRecent => r !== null)
  return {
    projects: projects.map((p) => ({ id: p.id, name: p.name, color: p.color })),
    sessions,
    recents,
    defaultProjectId: defaultProject(projects)
  }
}

// ---------------------------------------------------------------------------
// Making it

/** Near the middle of the board as the window last showed it, in the nearest free spot. */
export function spotOnBoard(board: BoardState, size: Size, windowSize: { w: number; h: number }): Point {
  const dock = dockRect(board.archive)
  const vp = board.viewport
  let preferred: Point
  if (vp && vp.zoom > 0) {
    preferred = {
      x: (windowSize.w / 2 - vp.x) / vp.zoom - size.w / 2,
      y: (windowSize.h / 2 - vp.y) / vp.zoom - size.h / 2
    }
  } else {
    const cards = Object.values(board.nodes).filter((n) => !n.archived && !n.runOf && !n.viewOf)
    const right = cards.reduce<(typeof cards)[number] | undefined>((a, b) => (a && a.x >= b.x ? a : b), undefined)
    preferred = right ? { x: right.x + cardSize(right).w + 32, y: right.y } : { x: dock.x + dock.w + 80, y: dock.y }
  }
  const spot = findFreeSpot(preferred, views.obstacles(board), dock, size)
  return { x: Math.round(spot.x), y: Math.round(spot.y) }
}

function windowSize() {
  const win = h!.mainWindow()
  const [w, ht] = win && !win.isDestroyed() ? win.getContentSize() : [1440, 920]
  return { w, h: ht }
}

const str = (v: unknown, max: number) => (typeof v === 'string' ? v.slice(0, max) : '')

/** Attachments from the page, kept to their known shapes. */
function cleanAttachments(raw: unknown): CaptureAttachment[] {
  if (!Array.isArray(raw)) return []
  const out: CaptureAttachment[] = []
  for (const a of raw.slice(0, 20) as Record<string, unknown>[]) {
    if (!a || typeof a !== 'object') continue
    if (a.kind === 'tab' && /^https?:\/\//i.test(str(a.url, 4000)))
      out.push({ kind: 'tab', url: str(a.url, 4000), title: str(a.title, 500), app: str(a.app, 100) })
    else if (a.kind === 'image' && str(a.path, 4000))
      out.push({ kind: 'image', path: str(a.path, 4000), thumb: '', copied: a.copied === true })
    else if (a.kind === 'file' && str(a.path, 4000))
      out.push({ kind: 'file', path: str(a.path, 4000), name: str(a.name, 500), size: Number(a.size) || 0 })
    else if (a.kind === 'text' && str(a.text, MAX_TEXT_CHARS).trim())
      out.push({ kind: 'text', text: str(a.text, MAX_TEXT_CHARS) })
  }
  return out
}

function remember(receipt: CaptureReceipt) {
  const state = getState()
  const record: CaptureRecord = {
    id: crypto.randomUUID(),
    kind: receipt.kind,
    projectId: receipt.projectId,
    title: receipt.title,
    createdAt: Date.now(),
    sessionId: receipt.sessionId,
    noteId: receipt.noteId,
    viewId: receipt.viewId
  }
  state.captures = [record, ...(state.captures ?? [])].slice(0, MAX_RECORDS)
  save()
}

export async function submit(raw: CaptureSubmission): Promise<CaptureReceipt> {
  const text = str(raw?.text, MAX_TEXT_CHARS).trim()
  if (!text) throw new Error('Type something first')
  if (!isCaptureKind(raw.kind)) throw new Error('Pick what to make')
  const project = getProject(raw.projectId)
  const board = getBoard(project.id)
  const attachments = cleanAttachments(raw.attachments)
  const prompt = withScreen(text, attachments)
  const base = { kind: raw.kind, projectId: project.id, projectName: project.name }
  let receipt: CaptureReceipt

  switch (raw.kind) {
    case 'session': {
      // Like the prompt bar's: expanded in a free spot, in auto mode, with the words stuck on it.
      const pos = spotOnBoard(board, cardSize({ archived: false, expanded: true }), windowSize())
      const card = await h!.createSession(project.id, { ...pos, initialPrompt: prompt, expanded: true, autoMode: true })
      const note: BoardNote = {
        id: crypto.randomUUID(),
        text,
        color: 'yellow',
        ...pos,
        sessionId: card.sessionId,
        createdAt: Date.now()
      }
      getBoard(project.id).notes![note.id] = note
      save()
      receipt = { ...base, title: titleFrom(text), sessionId: card.sessionId }
      break
    }
    case 'into': {
      const sessionId = str(raw.sessionId, 100)
      if (!sessionId || !sessionInfo(project.id, sessionId)) throw new Error('That session is no longer on the board')
      await h!.sendToSession(project.id, sessionId, prompt)
      receipt = { ...base, title: h!.sessionTitle(project.id, sessionId), sessionId }
      break
    }
    case 'note': {
      const size = fitNote(prompt)
      const note: BoardNote = {
        id: crypto.randomUUID(),
        text: prompt,
        color: 'yellow',
        ...spotOnBoard(board, size, windowSize()),
        w: size.w,
        h: size.h,
        createdAt: Date.now()
      }
      board.notes![note.id] = note
      save()
      receipt = { ...base, title: titleFrom(text), noteId: note.id }
      break
    }
    case 'view': {
      const pos = spotOnBoard(board, { w: VIEW_W, h: VIEW_H }, windowSize())
      const view = await views.createView(project.id, { description: prompt, ...pos })
      receipt = { ...base, title: titleFrom(text), viewId: view.id }
      break
    }
  }
  remember(receipt)
  h!.boardChanged(project.id)
  return receipt
}

async function describeFiles(paths: string[]): Promise<CaptureAttachment[]> {
  const out: CaptureAttachment[] = []
  for (const file of paths.slice(0, 20)) {
    try {
      const stat = await fs.promises.stat(file)
      if (stat.isFile() || stat.isDirectory())
        out.push({ kind: 'file', path: file, name: path.basename(file), size: stat.isFile() ? stat.size : 0 })
    } catch {
      // Gone already.
    }
  }
  return out
}

// ---------------------------------------------------------------------------

export function registerCapture(handlers: CaptureHandlers) {
  h = handlers
  pruneShots()

  ipcMain.handle(IPC.captureReady, () => (isPanelOpen() ? shownContext : null))

  ipcMain.handle(IPC.captureTargets, () => {
    const t = targets()
    if (openedOn) openedOn.targets = t
    return t
  })

  ipcMain.handle(IPC.captureGuess, (_e, text: unknown, attachments: unknown) => {
    const t = openedOn?.targets ?? targets()
    return guessCapture({
      text: str(text, MAX_TEXT_CHARS),
      attachments: cleanAttachments(attachments),
      app: openedOn?.app,
      targets: t
    })
  })

  ipcMain.handle(IPC.captureSubmit, async (_e, submission: CaptureSubmission) => {
    const receipt = await submit(submission)
    rest()
    return receipt
  })

  ipcMain.handle(IPC.captureScreenshot, () =>
    aside(async (win) => {
      win.hide()
      // Let the panel leave the screen before the shot.
      await new Promise((r) => setTimeout(r, 150))
      return takeScreenshot()
    })
  )

  ipcMain.handle(IPC.captureFiles, async (_e, paths: unknown) => {
    if (Array.isArray(paths))
      return describeFiles(paths.filter((p): p is string => typeof p === 'string' && path.isAbsolute(p)))
    return aside(async (win) => {
      const picked = await dialog.showOpenDialog(win, { properties: ['openFile', 'multiSelections'] })
      return picked.canceled ? [] : describeFiles(picked.filePaths)
    })
  })

  ipcMain.handle(IPC.captureClipboard, () => takeClipboard())

  ipcMain.on(IPC.captureHide, () => hidePanel())

  ipcMain.on(IPC.capturePassThrough, (_e, through: unknown) => {
    if (panel && !panel.isDestroyed()) panel.setIgnoreMouseEvents(through === true, { forward: true })
  })

  ipcMain.handle(IPC.captureOpen, (_e, request: OpenCardRequest) => {
    hidePanel()
    if (request && typeof request.projectId === 'string')
      h!.openCard({ projectId: request.projectId, sessionId: request.sessionId })
  })

  ipcMain.handle(IPC.captureSettings, () => {
    hidePanel()
    h!.openSettings()
  })

  ipcMain.handle(IPC.captureShortcut, () => shortcutState())

  ipcMain.handle(IPC.captureAllow, () => askPermission())

  ipcMain.handle(IPC.captureShow, () => openPanel())

  syncShortcut()
  app.on('will-quit', stopCapture)
}
