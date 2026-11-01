// Tells the user, while Nodus is in the background, when a session in any
// project needs them or finishes its turn, and counts the sessions waiting on
// them on the Dock icon. Clicking a notification opens its card.
import { app, Notification, type BrowserWindow } from 'electron'
import type { OpenCardRequest, SessionStatus } from '@shared/types'
import { readSession, transcriptPath } from './discovery'
import { getStatus } from './hooks'
import { isLive } from './pty'
import { getState, projectIdForSession } from './store'

export type Notice = 'waiting' | 'finished'

/** What a session's change of status is worth telling the user, if anything. */
export function noticeFor(prev: SessionStatus | undefined, status: SessionStatus, live: boolean): Notice | null {
  if (status === 'waiting') return prev === 'waiting' ? null : 'waiting'
  // Its turn ended. A process that went (closed, or its host lost) finished nothing.
  if (status === 'done' && prev === 'working' && live) return 'finished'
  return null
}

/**
 * Sessions waiting on the user across every project: the red cards, and
 * action runs with a question. The same ones the project folders count.
 */
export function waitingCount(): number {
  let count = 0
  for (const board of Object.values(getState().boards)) {
    for (const node of Object.values(board.nodes)) {
      // A view's brain isn't on the board.
      if (node.viewOf || node.archived || !isLive(node.sessionId)) continue
      if (getStatus(node.sessionId) === 'waiting') count++
    }
  }
  return count
}

interface Shown {
  notification: Notification
  notice: Notice
}

let getWindow: () => BrowserWindow | null = () => null
let showWindow: () => BrowserWindow = () => {
  throw new Error('No window')
}
let sendOpen: (request: OpenCardRequest) => void = () => {}
/** Hooks replay what happened while the app was closed; none of that is news. */
let armed = false
/** Each live session's last status, to tell what changed. */
const statuses = new Map<string, SessionStatus>()
/** The latest notification about each session, kept so its click still fires and it can be taken back. */
const shown = new Map<string, Shown>()
/** A card to open once a window that was closed has loaded again. */
let pendingOpen: OpenCardRequest | null = null
let badge = 0

export function configureNotify(opts: {
  window: () => BrowserWindow | null
  /** Brings the window to the front, opening it again if it was closed. */
  showWindow: () => BrowserWindow
  /** Asks the renderer to open a card. */
  open: (request: OpenCardRequest) => void
}) {
  getWindow = opts.window
  showWindow = opts.showWindow
  sendOpen = opts.open
}

/** Called once the sessions running before the app started have caught up. */
export function armNotifications() {
  armed = true
  updateBadge()
}

const inBackground = () => {
  const win = getWindow()
  return !win || win.isDestroyed() || !win.isFocused()
}

/** The card a notification about this session would open, and what to say about it. */
function describe(sessionId: string) {
  const projectId = projectIdForSession(sessionId)
  const project = getState().projects.find((p) => p.id === projectId)
  const node = projectId ? getState().boards[projectId]?.nodes[sessionId] : undefined
  if (!project || !node || node.viewOf || node.kind === 'terminal') return undefined
  const meta = readSession(sessionId, transcriptPath(project.repoPath, sessionId))?.meta
  return {
    projectId: project.id,
    projectName: project.name,
    title: node.title || meta?.title || 'New session',
    lastReply: meta?.lastAssistantText?.replace(/\s+/g, ' ').trim()
  }
}

function show(sessionId: string, notice: Notice) {
  const about = describe(sessionId)
  if (!about || !Notification.isSupported()) return
  shown.get(sessionId)?.notification.close()
  const notification = new Notification({
    title: about.title,
    subtitle: about.projectName,
    body: notice === 'waiting' ? 'Needs you' : about.lastReply || 'Finished',
    // Gathers a project's notifications in Notification Center.
    groupId: about.projectId
  })
  notification.on('click', () => {
    shown.delete(sessionId)
    // An action run leaves the board when it finishes, so its notification opens the board.
    const onBoard = projectIdForSession(sessionId) === about.projectId
    open({ projectId: about.projectId, sessionId: onBoard ? sessionId : undefined })
  })
  shown.set(sessionId, { notification, notice })
  notification.show()
}

/** Takes back a notification that no longer holds: one saying a session needs the user, once it doesn't. */
function withdraw(sessionId: string, notice: Notice) {
  const current = shown.get(sessionId)
  if (current?.notice !== notice) return
  current.notification.close()
  shown.delete(sessionId)
}

function open(request: OpenCardRequest) {
  if (!getState().projects.some((p) => p.id === request.projectId)) return void showWindow()
  const win = getWindow()
  const reopened = !win || win.isDestroyed() || win.webContents.isLoading()
  showWindow()
  // A window that's still loading picks the request up itself (takeOpenRequest).
  if (reopened) pendingOpen = request
  else sendOpen(request)
}

/** The card a notification asked to open before the window had loaded, if any. */
export function takeOpenRequest(): OpenCardRequest | null {
  const request = pendingOpen
  pendingOpen = null
  return request
}

/** Every change of a session's status passes through here. */
export function statusChanged(sessionId: string, status: SessionStatus, live: boolean) {
  const prev = statuses.get(sessionId)
  if (live) statuses.set(sessionId, status)
  else statuses.delete(sessionId)
  if (prev === 'waiting' && status !== 'waiting') withdraw(sessionId, 'waiting')
  updateBadge()

  const notice = noticeFor(prev, status, live)
  if (notice && armed && getState().settings.notify && inBackground()) show(sessionId, notice)
}

/** Shows how many sessions are waiting on the user on the Dock icon, when that's on. */
export function updateBadge() {
  if (!armed || !app.dock) return
  const count = getState().settings.dockBadge ? waitingCount() : 0
  if (count === badge) return
  badge = count
  app.dock.setBadge(count ? String(count) : '')
}
