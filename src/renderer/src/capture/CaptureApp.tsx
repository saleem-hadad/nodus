import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent,
  type KeyboardEvent
} from 'react'
import clsx from 'clsx'
import {
  AlertCircle,
  Check,
  ChevronDown,
  CornerDownRight,
  FileText,
  Globe,
  LayoutDashboard,
  Loader2,
  Mic,
  Paperclip,
  Plus,
  Scan,
  Square,
  SquareTerminal,
  X
} from 'lucide-react'
import { CAPTURE_KINDS } from '@shared/capture'
import type {
  CaptureAttachment,
  CaptureContext,
  CaptureGuess,
  CaptureKind,
  CaptureReceipt,
  CaptureRecent,
  CaptureSession,
  CaptureTargets,
  FolderColor
} from '@shared/types'
import { FOLDER_HUES } from '@renderer/components/FolderIcon'
import { useVoice } from '@renderer/components/voice/useVoice'
import { VoiceNotice, VoiceStatus } from '@renderer/components/voice/VoiceParts'
import { keys } from '@renderer/platform'
import { useApp } from '@renderer/store'
import '@renderer/components/voice/voice.css'
import './capture.css'

// The board window has the Settings dialog; a missing voice key opens it there.
useApp.setState({ openSettings: () => void window.api.capture.openSettings() })

/** How long typing has to pause before Claude is asked again. */
const GUESS_AFTER_MS = 700
/** Shorter than this, there's nothing to guess from yet. */
const MIN_GUESS_CHARS = 3
/** How long the receipt shows before the panel goes. */
const RECEIPT_MS = 2600
/** How often the recents' status is read while the panel is up. */
const POLL_MS = 2000
const MAX_HISTORY = 50
const MAX_ROWS = 8
/** Room under the open menu for its shadow. */
const MENU_ROOM = 48

const KINDS: Record<CaptureKind, { label: string; verb: string; hint: string }> = {
  session: { label: 'New session', verb: 'Start session', hint: 'Claude starts on it, in auto mode' },
  into: { label: 'Send into a session', verb: 'Send', hint: 'Adds to its conversation' },
  note: { label: 'Note', verb: 'Put up note', hint: 'A sticky on the board, nothing runs' },
  view: { label: 'View', verb: 'Build view', hint: 'A live page Claude keeps up to date' }
}

interface Route {
  kind: CaptureKind
  projectId: string | null
  sessionId?: string
}

type Offer = NonNullable<CaptureContext['clipboard']>

/** A receipt, with the words that went. */
type Shown = CaptureReceipt & { text: string }

// ---- small pieces ------------------------------------------------------------

const store = {
  get<T>(key: string, fallback: T): T {
    try {
      const raw = localStorage.getItem(key)
      return raw === null ? fallback : (JSON.parse(raw) as T)
    } catch {
      return fallback
    }
  },
  set(key: string, value: unknown) {
    try {
      if (value === undefined) localStorage.removeItem(key)
      else localStorage.setItem(key, JSON.stringify(value))
    } catch {
      // Private storage: nothing kept.
    }
  }
}

/** Something that tells one clipboard apart from the next, to offer it only once. */
const offerKey = (offer: Offer) =>
  offer.kind === 'text' ? `t:${offer.text.slice(0, 2000)}` : `i:${offer.thumb.slice(-120)}`

const sendable = (s: CaptureSession) => s.status !== 'waiting'

const host = (url: string) => {
  try {
    return new URL(url).host.replace(/^www\./, '')
  } catch {
    return url
  }
}

const bytes = (n: number) =>
  n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${Math.round(n / 1024)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`

const ago = (at: number) => {
  const min = Math.round((Date.now() - at) / 60_000)
  if (min < 1) return 'now'
  if (min < 60) return `${min}m`
  const h = Math.round(min / 60)
  return h < 24 ? `${h}h` : `${Math.round(h / 24)}d`
}

const clip = (text: string, max: number) => (text.length > max ? `${text.slice(0, max - 1).trimEnd()}…` : text)

function Folder({ color }: { color?: FolderColor }) {
  return <span className="cap-folder" style={{ background: FOLDER_HUES[color ?? 'blue'].frontBottom }} aria-hidden />
}

function KindIcon({ kind, size = 14 }: { kind: CaptureKind; size?: number }) {
  switch (kind) {
    case 'session':
      return <SquareTerminal size={size} aria-hidden />
    case 'into':
      return <CornerDownRight size={size} className="cap-into-ink" aria-hidden />
    case 'note':
      return <span className="cap-note-swatch" style={{ width: size - 2, height: size - 2 }} aria-hidden />
    case 'view':
      return <LayoutDashboard size={size} className="cap-view-ink" aria-hidden />
  }
}

/** A status dot, or a note's or view's square. */
function Mark({ recent }: { recent: Pick<CaptureRecent, 'kind' | 'status'> }) {
  if (recent.kind === 'note') return <span className="cap-mark cap-mark-note" aria-hidden />
  if (recent.kind === 'view') return <span className="cap-mark cap-mark-view" aria-hidden />
  return <span className={clsx('cap-mark', 'cap-dot', `is-${recent.status ?? 'done'}`)} aria-hidden />
}

const statusLabel = (recent: CaptureRecent) =>
  recent.kind === 'note'
    ? 'Note'
    : recent.kind === 'view'
      ? 'View'
      : recent.status === 'waiting'
        ? 'Needs you'
        : recent.status === 'working'
          ? 'Working'
          : 'Done'

// ---- the panel ---------------------------------------------------------------

/**
 * The floating capture panel (#capture): what's typed becomes a session, a
 * note or a view on a project's board, or goes into a session already there.
 * Claude guesses which after each pause in the typing, and the header says
 * where it'll go; Tab or a click changes it.
 */
export function CaptureApp() {
  const [targets, setTargets] = useState<CaptureTargets | null>(null)
  const [context, setContext] = useState<CaptureContext>({ shortcut: 'ready' })
  const [text, setText] = useState(() => store.get('capture.draft', ''))
  const [attachments, setAttachments] = useState<CaptureAttachment[]>(() => store.get('capture.attachments', []))
  const [guess, setGuess] = useState<CaptureGuess | null>(null)
  const [guessing, setGuessing] = useState(false)
  /** What the user picked, which stands over Claude's guess until the text is cleared. */
  const [choice, setChoice] = useState<Route | null>(null)
  const [menuOpen, setMenuOpen] = useState(false)
  const [sending, setSending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  /** The last screenshot was kept from Nodus: it has no Screen Recording permission. */
  const [shotBlocked, setShotBlocked] = useState(false)
  const [receipt, setReceipt] = useState<Shown | null>(null)
  const [dragging, setDragging] = useState(false)
  const [offerDone, setOfferDone] = useState<string | null>(() => store.get('capture.clipboardDone', null))
  const [history] = useState<string[]>(() => store.get('capture.history', []))
  /** Where ↑ is in the history; -1 while typing. */
  const [historyAt, setHistoryAt] = useState(-1)

  const fieldRef = useRef<HTMLTextAreaElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const routeRef = useRef<HTMLButtonElement>(null)
  /** Bumped by every guess asked, so a slow answer to old text is dropped. */
  const guessRun = useRef(0)
  /** The text the guess on screen was for. */
  const guessedFor = useRef('')
  const pendingGuess = useRef<Promise<CaptureGuess | null> | null>(null)
  const receiptTimer = useRef(0)
  /** The height the window was last asked to be. */
  const sentHeight = useRef(0)

  const projects = targets?.projects ?? []
  const sessions = targets?.sessions ?? []
  const projectOf = (id: string | null | undefined) => projects.find((p) => p.id === id)
  const sessionOf = (id: string | undefined) => sessions.find((s) => s.sessionId === id)

  const fallback: Route = { kind: 'session', projectId: targets?.defaultProjectId ?? projects[0]?.id ?? null }
  const route: Route =
    choice ?? (guess ? { kind: guess.kind, projectId: guess.projectId, sessionId: guess.sessionId } : fallback)
  const project = projectOf(route.projectId)
  const target = route.kind === 'into' ? sessionOf(route.sessionId) : undefined
  const hasText = text.trim().length > 0
  const offer = context.clipboard && offerKey(context.clipboard) !== offerDone ? context.clipboard : undefined

  const voice = useVoice(
    route.projectId ?? '',
    (said) => {
      setText((t) => (t.trim() ? `${t.trimEnd()} ${said}` : said))
      fieldRef.current?.focus()
    },
    { field: fieldRef }
  )

  // ---- the panel coming up and going -----------------------------------------

  const refresh = useCallback(() => {
    void window.api.capture.targets().then(setTargets)
  }, [])

  useEffect(() => {
    document.documentElement.classList.add('capture-window')
    const shown = (ctx: CaptureContext) => {
      window.clearTimeout(receiptTimer.current)
      setReceipt(null)
      setMenuOpen(false)
      setError(null)
      setShotBlocked(false)
      setContext(ctx)
      refresh()
      // Typing starts the moment the panel is up, so the field takes the keyboard now, and again once it has drawn.
      const focus = () => {
        const field = fieldRef.current
        if (!field || document.activeElement === field) return
        field.focus()
        field.setSelectionRange(field.value.length, field.value.length)
      }
      focus()
      requestAnimationFrame(focus)
    }
    const offShown = window.api.on.captureShown(shown)
    // The panel came up while this page was still loading.
    void window.api.capture.ready().then((ctx) => ctx && shown(ctx))
    const offContext = window.api.on.captureContext((ctx) => setContext((c) => ({ ...c, ...ctx })))
    return () => {
      offShown()
      offContext()
    }
  }, [refresh])

  // The browser tab in front comes along, unless a draft already has one.
  useEffect(() => {
    const tab = context.tab
    if (!tab) return
    setAttachments((list) => (list.some((a) => a.kind === 'tab') ? list : [tab, ...list]))
  }, [context.tab])

  // The recents' status, kept up to date while the panel is up.
  useEffect(() => {
    const tick = window.setInterval(() => {
      if (document.visibilityState === 'visible') refresh()
    }, POLL_MS)
    return () => window.clearInterval(tick)
  }, [refresh])

  // The draft outlives the panel and the app.
  useEffect(() => store.set('capture.draft', text || undefined), [text])
  useEffect(() => store.set('capture.attachments', attachments.length ? attachments : undefined), [attachments])

  // The window is as tall as what it shows, the open menu included, so a click
  // below the card lands in the app there rather than in an empty window.
  useLayoutEffect(() => {
    const stage = document.querySelector<HTMLElement>('.cap-stage')
    if (!stage) return
    const fit = () => {
      let bottom = stage.offsetTop + stage.offsetHeight
      for (const menu of stage.querySelectorAll<HTMLElement>('.cap-menu')) {
        bottom = Math.max(bottom, menu.getBoundingClientRect().bottom + MENU_ROOM)
      }
      const height = Math.ceil(bottom)
      if (height === sentHeight.current) return
      sentHeight.current = height
      window.api.capture.resize(height)
    }
    fit()
    const watch = new ResizeObserver(fit)
    watch.observe(stage)
    return () => watch.disconnect()
  })

  // The menu closes on a click anywhere but itself and the header that opens it.
  useEffect(() => {
    if (!menuOpen) return
    const onDown = (e: PointerEvent) => {
      const hit = e.target as Node
      if (!menuRef.current?.contains(hit) && !routeRef.current?.contains(hit)) setMenuOpen(false)
    }
    window.addEventListener('pointerdown', onDown, true)
    return () => window.removeEventListener('pointerdown', onDown, true)
  }, [menuOpen])

  // The field grows with what's written, up to a point.
  useLayoutEffect(() => {
    const field = fieldRef.current
    if (!field) return
    field.style.height = 'auto'
    const line = parseFloat(getComputedStyle(field).lineHeight) || 22
    field.style.height = `${Math.min(field.scrollHeight, line * MAX_ROWS + 24)}px`
  }, [text, receipt])

  // ---- Claude's guess ----------------------------------------------------------

  const askGuess = useCallback((forText: string, withAttachments: CaptureAttachment[]) => {
    const run = ++guessRun.current
    setGuessing(true)
    const pending = window.api.capture.guess(forText, withAttachments).catch(() => null)
    pendingGuess.current = pending
    void pending.then((g) => {
      if (run !== guessRun.current) return
      pendingGuess.current = null
      setGuessing(false)
      if (g) {
        guessedFor.current = forText.trim()
        setGuess(g)
      }
    })
    return pending
  }, [])

  // Each pause in the typing asks again, until the user picks for themselves.
  useEffect(() => {
    const trimmed = text.trim()
    if (!trimmed) {
      guessRun.current++
      guessedFor.current = ''
      pendingGuess.current = null
      setGuess(null)
      setGuessing(false)
      setChoice(null)
      return
    }
    if (choice || trimmed.length < MIN_GUESS_CHARS || trimmed === guessedFor.current) return
    const timer = window.setTimeout(() => void askGuess(text, attachments), GUESS_AFTER_MS)
    return () => window.clearTimeout(timer)
  }, [text, attachments, choice, askGuess])

  // ---- choosing for yourself ---------------------------------------------------------------

  /** A session to send into in a project: the one already picked, else the most recent that isn't waiting. */
  const intoSession = (projectId: string | null, current?: string): string | undefined => {
    const inProject = sessions.filter((s) => s.projectId === projectId && sendable(s))
    return inProject.find((s) => s.sessionId === current)?.sessionId ?? inProject[0]?.sessionId
  }

  const pickKind = (kind: CaptureKind) => {
    if (kind === 'into') {
      let projectId = route.projectId
      let sessionId = intoSession(projectId, route.sessionId)
      // None here: the most recent session anywhere.
      if (!sessionId) {
        const any = sessions.find(sendable)
        if (!any) return
        projectId = any.projectId
        sessionId = any.sessionId
      }
      setChoice({ kind, projectId, sessionId })
    } else {
      setChoice({ kind, projectId: route.projectId })
    }
    setError(null)
  }

  const cycleKind = (step: 1 | -1) => {
    const canInto = sessions.some(sendable)
    const order = CAPTURE_KINDS.filter((k) => k !== 'into' || canInto)
    const at = order.indexOf(route.kind)
    pickKind(order[(at + step + order.length) % order.length])
  }

  const pickProject = (projectId: string) => {
    if (route.kind === 'into') {
      const sessionId = intoSession(projectId)
      setChoice(sessionId ? { kind: 'into', projectId, sessionId } : { kind: 'session', projectId })
    } else {
      setChoice({ kind: route.kind, projectId })
    }
    setMenuOpen(false)
    fieldRef.current?.focus()
  }

  const pickSession = (s: CaptureSession) => {
    setChoice({ kind: 'into', projectId: s.projectId, sessionId: s.sessionId })
    setMenuOpen(false)
    fieldRef.current?.focus()
  }

  // ---- what comes along -----------------------------------------------------------

  const attach = (more: CaptureAttachment[]) => {
    if (more.length) setAttachments((list) => [...list, ...more])
    fieldRef.current?.focus()
  }

  const detach = (index: number) => {
    setAttachments((list) => list.filter((_, i) => i !== index))
    fieldRef.current?.focus()
  }

  const takeOffer = async () => {
    if (!offer) return
    const taken = await window.api.capture.clipboard()
    setOfferDone(offerKey(offer))
    store.set('capture.clipboardDone', offerKey(offer))
    if (taken) attach([taken])
  }

  const screenshot = async () => {
    const shot = await window.api.capture.screenshot()
    setShotBlocked(shot === 'needs-permission')
    if (shot && shot !== 'needs-permission') attach([shot])
    else fieldRef.current?.focus()
  }

  const pickFiles = async () => attach(await window.api.capture.files())

  const onDrop = async (e: DragEvent) => {
    e.preventDefault()
    setDragging(false)
    const paths = [...e.dataTransfer.files].map((f) => window.api.pathForFile(f)).filter(Boolean)
    if (paths.length) attach(await window.api.capture.files(paths))
  }

  // ---- sending ----------------------------------------------------------------

  const send = async () => {
    const trimmed = text.trim()
    if (!trimmed || sending) return
    setSending(true)
    setError(null)
    setMenuOpen(false)
    try {
      let to = route
      // Typed and sent before Claude had a look: wait for its guess.
      if (!choice && guessedFor.current !== trimmed) {
        const g = await (pendingGuess.current ?? askGuess(text, attachments))
        if (g) to = { kind: g.kind, projectId: g.projectId, sessionId: g.sessionId }
      }
      if (!to.projectId) throw new Error('Add a project in Nodus first')
      if (to.kind === 'into' && !to.sessionId) throw new Error('There’s no session to send into. Pick another kind.')
      const done = await window.api.capture.submit({
        text: trimmed,
        kind: to.kind,
        projectId: to.projectId,
        sessionId: to.kind === 'into' ? to.sessionId : undefined,
        attachments
      })
      const kept = [trimmed, ...history.filter((h) => h !== trimmed)].slice(0, MAX_HISTORY)
      history.splice(0, history.length, ...kept)
      store.set('capture.history', kept)
      // What was on the clipboard was seen, taken or not.
      if (context.clipboard) {
        setOfferDone(offerKey(context.clipboard))
        store.set('capture.clipboardDone', offerKey(context.clipboard))
      }
      setText('')
      setAttachments([])
      setChoice(null)
      setHistoryAt(-1)
      setReceipt({ ...done, text: trimmed })
      refresh()
      receiptTimer.current = window.setTimeout(() => {
        setReceipt(null)
        window.api.capture.hide()
      }, RECEIPT_MS)
    } catch (err) {
      setError(
        err instanceof Error
          ? err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
          : String(err)
      )
    } finally {
      setSending(false)
    }
  }

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing) return
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      void send()
    } else if (e.key === 'Tab') {
      e.preventDefault()
      if (hasText) cycleKind(e.shiftKey ? -1 : 1)
    } else if (e.key === 'Escape') {
      e.preventDefault()
      if (menuOpen) setMenuOpen(false)
      else window.api.capture.hide()
    } else if (e.key === 'ArrowUp' && (!hasText || historyAt >= 0) && history.length) {
      // Earlier captures, newest first.
      e.preventDefault()
      const at = Math.min(historyAt + 1, history.length - 1)
      setHistoryAt(at)
      setText(history[at])
    } else if (e.key === 'ArrowDown' && historyAt >= 0) {
      e.preventDefault()
      const at = historyAt - 1
      setHistoryAt(at)
      setText(at >= 0 ? history[at] : '')
    }
  }

  const open = (request: { projectId: string; sessionId?: string }) => {
    window.clearTimeout(receiptTimer.current)
    setReceipt(null)
    void window.api.capture.open(request)
  }

  // ---- what it shows -------------------------------------------------------------

  const listening = voice.phase !== 'idle'
  const recents = targets?.recents ?? []
  const noProjects = targets !== null && projects.length === 0

  const header = () => {
    if (!hasText || !targets) {
      return (
        <>
          <Folder color={project?.color} />
          <span>{project?.name ?? 'No projects yet'}</span>
          <ChevronDown size={14} className="cap-faint" aria-hidden />
        </>
      )
    }
    if (guessing && !guess && !choice) {
      return (
        <span className="cap-thinking">
          <Loader2 size={14} className="cap-spin" aria-hidden />
          Working out where this goes…
        </span>
      )
    }
    return (
      <>
        <span className="cap-kind-badge">
          <KindIcon kind={route.kind} />
        </span>
        {route.kind === 'into' ? (
          <strong className="cap-route-main">Into “{target?.title ?? 'a session'}”</strong>
        ) : (
          <>
            <strong>{route.kind === 'session' ? 'New session' : route.kind === 'note' ? 'Note' : 'View'}</strong>
            <span className="cap-faint">{route.kind === 'note' ? 'on' : 'in'}</span>
            <Folder color={project?.color} />
            <span>{project?.name}</span>
          </>
        )}
        <ChevronDown size={14} className="cap-faint" aria-hidden />
      </>
    )
  }

  if (receipt) return <Receipt receipt={receipt} color={projectOf(receipt.projectId)?.color} onOpen={open} />

  return (
    <div className="cap-stage" ref={voice.rootRef}>
      <div
        className={clsx('cap-card', 'cap-hit', dragging && 'is-dropping')}
        role="dialog"
        aria-label="Nodus capture"
        onDragOver={(e) => {
          e.preventDefault()
          setDragging(true)
        }}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node)) setDragging(false)
        }}
        onDrop={(e) => void onDrop(e)}
      >
        <div className="cap-head">
          <button
            ref={routeRef}
            type="button"
            className={clsx('cap-route', menuOpen && 'is-open')}
            aria-haspopup="dialog"
            aria-expanded={menuOpen}
            disabled={!targets || noProjects}
            onClick={() => {
              setMenuOpen((o) => !o)
              fieldRef.current?.focus()
            }}
          >
            {header()}
          </button>
          <span className="cap-grow" />
          {hasText && (guess || choice) ? (
            <span className="cap-who">
              {choice ? (
                'Your choice'
              ) : (
                <>
                  <span className={clsx('cap-who-dot', guessing && 'is-thinking')} aria-hidden />
                  Claude's guess
                </>
              )}
              <kbd className="cap-kbd">Tab</kbd>
            </span>
          ) : (
            !hasText && <span className="cap-who">Claude works out what it is as you type</span>
          )}
        </div>

        <textarea
          ref={fieldRef}
          className="cap-field"
          aria-label="What to capture"
          placeholder={
            noProjects ? 'Add a project in Nodus to capture into' : 'Ask Claude to do something, or jot a note'
          }
          rows={3}
          autoFocus
          value={text}
          spellCheck
          onChange={(e) => {
            setText(e.target.value)
            setHistoryAt(-1)
            setError(null)
          }}
          onKeyDown={onKeyDown}
        />

        {(attachments.length > 0 || offer) && (
          <div className="cap-tiles">
            {attachments.map((a, i) => (
              <Tile key={i} attachment={a} onRemove={() => detach(i)} />
            ))}
            {offer && (
              <button type="button" className="cap-tile cap-offer" onClick={() => void takeOffer()}>
                <span className="cap-tile-top">
                  <Plus size={12} aria-hidden />
                  {offer.kind === 'text' ? 'Copied text' : 'Copied image'}
                </span>
                {offer.kind === 'text' ? (
                  <span className="cap-mono cap-two-lines">{offer.text}</span>
                ) : (
                  <img className="cap-offer-thumb" src={offer.thumb} alt="" />
                )}
              </button>
            )}
          </div>
        )}

        {error && (
          <p className="cap-error" role="alert">
            {error}
          </p>
        )}
        {shotBlocked && (
          <div className="cap-notice" role="alert">
            <AlertCircle size={14} className="cap-notice-icon" aria-hidden />
            <span className="cap-notice-text">
              macOS kept the screenshot from Nodus. Turn Nodus on in Screen &amp; System Audio Recording, then take it
              again.
            </span>
            <button type="button" className="cap-banner-btn" onClick={() => void window.api.capture.allowScreen()}>
              Open System Settings
            </button>
          </div>
        )}
        <VoiceNotice voice={voice} />

        <div className="cap-foot">
          <button
            type="button"
            className="cap-icon"
            onClick={() => void screenshot()}
            title="Take a screenshot"
            aria-label="Take a screenshot"
          >
            <Scan size={16} />
          </button>
          <button
            type="button"
            className="cap-icon"
            onClick={() => void pickFiles()}
            title="Attach files"
            aria-label="Attach files"
          >
            <Paperclip size={16} />
          </button>
          <button
            type="button"
            className={clsx('cap-icon', voice.phase === 'recording' && 'is-recording')}
            onClick={() => (voice.phase === 'recording' ? void voice.stop() : void voice.record())}
            disabled={voice.phase === 'opening' || voice.phase === 'transcribing'}
            title={voice.phase === 'recording' ? 'Stop and transcribe' : 'Speak'}
            aria-label={voice.phase === 'recording' ? 'Stop and transcribe' : 'Speak'}
          >
            {voice.phase === 'recording' ? <Square size={13} fill="currentColor" /> : <Mic size={16} />}
          </button>
          <span className="cap-grow" />
          {listening ? (
            <VoiceStatus voice={voice} />
          ) : (
            <span className="cap-hint">
              <kbd className="cap-kbd">{keys('⇧↩')}</kbd> new line
            </span>
          )}
          <button
            type="button"
            className="cap-send"
            disabled={!hasText || sending || noProjects}
            onClick={() => void send()}
          >
            {sending ? <Loader2 size={14} className="cap-spin" aria-hidden /> : null}
            {sending && !choice && guessedFor.current !== text.trim() ? 'Working it out…' : KINDS[route.kind].verb}
            <kbd className="cap-send-key" aria-hidden>
              {keys('↩')}
            </kbd>
          </button>
        </div>

        {menuOpen && targets && (
          <RouteMenu
            ref={menuRef}
            route={route}
            reason={!choice && guess ? guess.reason : null}
            targets={targets}
            onKind={(k) => {
              pickKind(k)
              fieldRef.current?.focus()
            }}
            onProject={pickProject}
            onSession={pickSession}
          />
        )}
      </div>

      {!hasText && context.shortcut !== 'ready' && context.shortcut !== 'off' && (
        <div className="cap-banner cap-hit">
          {context.shortcut === 'needs-permission' ? (
            <>
              <span>
                To open this from any app with a double tap of ⇧, allow Nodus in Accessibility. Already on there? Click
                Allow… anyway.
              </span>
              <button
                type="button"
                className="cap-banner-btn"
                onClick={() =>
                  void window.api.capture.allow().then((shortcut) => setContext((c) => ({ ...c, shortcut })))
                }
              >
                Allow…
              </button>
            </>
          ) : (
            <span>The double tap of ⇧ couldn’t start on this computer. Use File › Capture… instead.</span>
          )}
        </div>
      )}

      {!hasText && recents.length > 0 && (
        <div className="cap-recents" aria-label="Recent captures">
          <div className="cap-recent-grid">
            {recents.slice(0, 3).map((r) => (
              <button
                key={r.id}
                type="button"
                className={clsx(
                  'cap-recent',
                  'cap-hit',
                  r.status === 'waiting' && r.kind !== 'note' && r.kind !== 'view' && 'is-waiting'
                )}
                onClick={() => open({ projectId: r.projectId, sessionId: r.sessionId })}
                title="Open it on the board"
              >
                <span className="cap-recent-top">
                  <Mark recent={r} />
                  <span className="cap-grow cap-ellipsis">{projectOf(r.projectId)?.name}</span>
                  <span>{ago(r.createdAt)}</span>
                </span>
                <span className="cap-recent-title">{r.title}</span>
                <span
                  className={clsx(
                    'cap-recent-status',
                    `is-${r.kind === 'note' || r.kind === 'view' ? 'plain' : r.status}`
                  )}
                >
                  {statusLabel(r)}
                </span>
              </button>
            ))}
          </div>
          {recents.length > 3 && (
            <p className="cap-more cap-hit">
              {recents.length - 3} more ·{' '}
              {recents
                .slice(3, 5)
                .map((r) => r.title)
                .join(', ')}
              <span className="cap-faint"> · ↑ for earlier captures</span>
            </p>
          )}
        </div>
      )}
    </div>
  )
}

// ---- attachments -------------------------------------------------------------

function Tile({ attachment: a, onRemove }: { attachment: CaptureAttachment; onRemove: () => void }) {
  const remove = (
    <button type="button" className="cap-tile-x" onClick={onRemove} aria-label="Remove" title="Remove">
      <X size={11} />
    </button>
  )
  switch (a.kind) {
    case 'tab':
      return (
        <div className="cap-tile cap-tile-tab" title={a.url}>
          <span className="cap-tile-top">
            <Globe size={12} aria-hidden />
            <span className="cap-ellipsis">{host(a.url)}</span>
          </span>
          <span className="cap-two-lines">{a.title || a.url}</span>
          {remove}
        </div>
      )
    case 'image':
      return (
        <div className="cap-tile cap-tile-image" title={a.path}>
          {a.thumb && <img src={a.thumb} alt="" />}
          <span className="cap-tile-label">{a.copied ? 'Copied image' : 'Screenshot'}</span>
          {remove}
        </div>
      )
    case 'file':
      return (
        <div className="cap-tile cap-tile-file" title={a.path}>
          <FileText size={18} className="cap-faint" aria-hidden />
          <span className="cap-tile-file-text">
            <span className="cap-ellipsis">{a.name}</span>
            {a.size > 0 && <span className="cap-faint">{bytes(a.size)}</span>}
          </span>
          {remove}
        </div>
      )
    case 'text':
      return (
        <div className="cap-tile cap-tile-text">
          <span className="cap-tile-top">Copied text</span>
          <span className="cap-mono cap-two-lines">{a.text}</span>
          {remove}
        </div>
      )
  }
}

// ---- where it goes -------------------------------------------------------------

interface MenuProps {
  route: Route
  reason: string | null
  targets: CaptureTargets
  onKind(kind: CaptureKind): void
  onProject(projectId: string): void
  onSession(session: CaptureSession): void
  ref: React.Ref<HTMLDivElement>
}

/** What to make on the left, where it goes on the right: projects, or for Into, the sessions. */
function RouteMenu({ route, reason, targets, onKind, onProject, onSession, ref }: MenuProps) {
  const project = targets.projects.find((p) => p.id === route.projectId)
  const canInto = targets.sessions.some(sendable)
  const here = targets.sessions.filter((s) => s.projectId === route.projectId).slice(0, 8)
  return (
    <div ref={ref} className="cap-menu cap-hit" role="dialog" aria-label="Where it goes">
      {reason && (
        <p className="cap-reason">
          <span className="cap-who-dot" aria-hidden />
          <span>
            <strong>Claude's guess:</strong> {reason}
          </span>
        </p>
      )}
      <div className="cap-menu-cols">
        <div className="cap-menu-col" role="listbox" aria-label="What to make">
          <span className="cap-menu-label">Make</span>
          {CAPTURE_KINDS.map((k) => (
            <button
              key={k}
              type="button"
              role="option"
              aria-selected={route.kind === k}
              className={clsx('cap-option', route.kind === k && 'is-selected')}
              disabled={k === 'into' && !canInto}
              onClick={() => onKind(k)}
            >
              <span className="cap-option-icon">
                <KindIcon kind={k} size={16} />
              </span>
              <span className="cap-option-text">
                <span>{KINDS[k].label}</span>
                <span className="cap-faint">
                  {k === 'into' && !canInto ? 'No sessions are running' : KINDS[k].hint}
                </span>
              </span>
            </button>
          ))}
        </div>
        <div
          className="cap-menu-col"
          role="listbox"
          aria-label={route.kind === 'into' ? 'Which session' : 'Which project'}
        >
          {route.kind === 'into' ? (
            <>
              <span className="cap-menu-label">
                <Folder color={project?.color} />
                {project?.name}
              </span>
              {here.map((s) => (
                <button
                  key={s.sessionId}
                  type="button"
                  role="option"
                  aria-selected={route.sessionId === s.sessionId}
                  className={clsx('cap-option', 'cap-option-row', route.sessionId === s.sessionId && 'is-selected')}
                  disabled={!sendable(s)}
                  title={sendable(s) ? undefined : 'It’s waiting for your answer. Answer it on the board first.'}
                  onClick={() => onSession(s)}
                >
                  <span className={clsx('cap-mark', 'cap-dot', `is-${s.status}`)} aria-hidden />
                  <span className="cap-grow cap-ellipsis">{s.title}</span>
                  {!sendable(s) ? (
                    <span className="cap-needs">Needs you</span>
                  ) : (
                    route.sessionId === s.sessionId && <Check size={14} className="cap-check" aria-hidden />
                  )}
                </button>
              ))}
              {here.length === 0 && <span className="cap-faint cap-menu-empty">No sessions on this board</span>}
              <span className="cap-menu-sep" />
              {targets.projects
                .filter((p) => p.id !== route.projectId)
                .map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    role="option"
                    aria-selected={false}
                    className="cap-option cap-option-row"
                    onClick={() => onProject(p.id)}
                  >
                    <Folder color={p.color} />
                    <span className="cap-grow cap-ellipsis">{p.name}</span>
                    <span className="cap-faint">
                      {targets.sessions.filter((s) => s.projectId === p.id).length || 'no'} sessions
                    </span>
                  </button>
                ))}
            </>
          ) : (
            <>
              <span className="cap-menu-label">Project</span>
              {targets.projects.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  role="option"
                  aria-selected={p.id === route.projectId}
                  className={clsx('cap-option', 'cap-option-row', p.id === route.projectId && 'is-selected')}
                  onClick={() => onProject(p.id)}
                >
                  <Folder color={p.color} />
                  <span className="cap-grow cap-ellipsis">{p.name}</span>
                  {p.id === route.projectId && <Check size={14} className="cap-check" aria-hidden />}
                </button>
              ))}
            </>
          )}
        </div>
      </div>
      <p className="cap-menu-foot">
        <span>
          <kbd className="cap-kbd">Tab</kbd> next kind
        </span>
        <span>
          <kbd className="cap-kbd">Esc</kbd> close
        </span>
      </p>
    </div>
  )
}

// ---- the receipt -------------------------------------------------------------

function Receipt({
  receipt,
  color,
  onOpen
}: {
  receipt: Shown
  color?: FolderColor
  onOpen(request: { projectId: string; sessionId?: string }): void
}) {
  const openIt = () => onOpen({ projectId: receipt.projectId, sessionId: receipt.sessionId })
  if (receipt.kind === 'note') {
    return (
      <div className="cap-stage cap-receipt-stage">
        <div
          className="cap-receipt-note cap-hit"
          role="status"
          style={{ '--note-hue': FOLDER_HUES.yellow.frontTop } as CSSProperties}
        >
          <span className="cap-receipt-note-text">{clip(receipt.text, 160)}</span>
        </div>
        <p className="cap-receipt-caption cap-hit">
          On {receipt.projectName}’s board ·{' '}
          <button type="button" className="cap-link" onClick={openIt}>
            Open
          </button>
        </p>
      </div>
    )
  }
  const view = receipt.kind === 'view'
  return (
    <div className="cap-stage cap-receipt-stage">
      <div className={clsx('cap-receipt', 'cap-hit', view && 'is-view')} role="status">
        <span className="cap-receipt-top">
          {view ? (
            <>
              <Loader2 size={12} className="cap-spin cap-view-ink" aria-hidden />
              <span className="cap-view-ink">Building view</span>
            </>
          ) : (
            <>
              <span className="cap-mark cap-dot is-working" aria-hidden />
              <span className="cap-working-ink">{receipt.kind === 'into' ? 'Sent' : 'Working'}</span>
            </>
          )}
        </span>
        <span className={clsx('cap-receipt-title', receipt.kind === 'session' && 'has-sticker')}>{receipt.title}</span>
        <span className="cap-faint cap-receipt-line">
          {receipt.kind === 'session'
            ? 'Claude is starting on it, expanded on the board.'
            : receipt.kind === 'into'
              ? 'Your message is in. Claude is on it.'
              : 'Its brain is putting it together.'}
        </span>
        <span className="cap-receipt-foot">
          <button type="button" className="cap-link" onClick={openIt}>
            Open on board
          </button>
          <span className="cap-receipt-where">
            <Folder color={color} />
            {receipt.projectName}
            {receipt.kind === 'session' && <span className="cap-faint">· auto mode</span>}
          </span>
        </span>
        {/* A new session wears what was asked, as its card does on the board. */}
        {receipt.kind === 'session' && (
          <span
            className="cap-sticker"
            style={{ '--note-hue': FOLDER_HUES.yellow.frontTop } as CSSProperties}
            aria-hidden
          >
            {clip(receipt.text, 70)}
          </span>
        )}
      </div>
    </div>
  )
}
