import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { ArchiveRestore, X } from 'lucide-react'
import { Highlight } from '@renderer/components/Highlight'
import { Modal } from '@renderer/components/Modal'
import { cardTitle, useApp, useCard, type PreviewFind } from '@renderer/store'
import { STACK_ROWS, stackZone } from '@renderer/components/board/layout'
import type { BoardSnapshot, SearchSnippet, ToolCall, TranscriptMessage } from '@shared/types'
import { matchRanges, queryTerms } from '@shared/search'
import './terminal.css'

const CARD_W = 288
const CARD_H = 216
const GAP = 40

export function TranscriptPreview() {
  const sessionId = useApp((s) => s.previewSessionId)
  const projectId = useApp((s) => s.board?.project.id)
  const closePreview = useApp((s) => s.closePreview)

  return (
    <Modal
      open={Boolean(sessionId && projectId)}
      onClose={closePreview}
      className="tp-modal"
      labelledBy="tp-title"
    >
      {sessionId && projectId && (
        <PreviewPanel key={sessionId} sessionId={sessionId} projectId={projectId} />
      )}
    </Modal>
  )
}

type Load =
  | { state: 'loading' }
  | { state: 'error'; message: string }
  | { state: 'ready'; messages: TranscriptMessage[] }

function PreviewPanel({ sessionId, projectId }: { sessionId: string; projectId: string }) {
  const card = useCard(sessionId)
  const closePreview = useApp((s) => s.closePreview)
  const find = useApp((s) => s.previewFind)
  const terms = useMemo(() => queryTerms(find?.query ?? ''), [find])
  const [load, setLoad] = useState<Load>({ state: 'loading' })
  const scrollRef = useRef<HTMLDivElement>(null)

  const fetchTranscript = useCallback(() => {
    let cancelled = false
    setLoad({ state: 'loading' })
    window.api.sessions.transcript(projectId, sessionId).then(
      (messages) => !cancelled && setLoad({ state: 'ready', messages }),
      (err: unknown) =>
        !cancelled &&
        setLoad({ state: 'error', message: err instanceof Error ? err.message : String(err) })
    )
    return () => {
      cancelled = true
    }
  }, [projectId, sessionId])

  useEffect(fetchTranscript, [fetchTranscript])

  const found = useMemo(() => (load.state === 'ready' && find ? foundIndex(load.messages, find, terms) : -1), [load, find, terms])

  // Opened from a search, the match is in view; otherwise the latest messages are.
  useLayoutEffect(() => {
    if (load.state !== 'ready' || !scrollRef.current) return
    const target = found >= 0 && scrollRef.current.querySelector<HTMLElement>(`[data-index="${found}"]`)
    // A long turn is centred on its first marked word, not its middle.
    if (target) (target.querySelector('mark') ?? target).scrollIntoView({ block: 'center' })
    else scrollRef.current.scrollTop = scrollRef.current.scrollHeight
  }, [load, found])

  const restore = () => {
    const { board, patchCards, closePreview, openTerminal } = useApp.getState()
    if (!board) return
    const { x, y } = freeSlotBesideArchive(board, sessionId)
    patchCards([{ sessionId, archived: false, x, y }])
    closePreview()
    openTerminal(sessionId)
  }

  const meta = card?.meta
  const messageCount = load.state === 'ready' ? load.messages.length : meta?.messageCount

  return (
    <>
      <header className="tp-header">
        <div className="tp-heading">
          <h2 id="tp-title" className="tp-title">
            {cardTitle(card)}
          </h2>
          <div className="tp-sub">
            {meta && <span>{formatRange(meta.createdAt, meta.updatedAt)}</span>}
            {messageCount !== undefined && (
              <span>
                {messageCount} {messageCount === 1 ? 'message' : 'messages'}
              </span>
            )}
            {meta?.gitBranch && <span className="tp-mono">{meta.gitBranch}</span>}
          </div>
        </div>
        <div className="tp-actions">
          <button className="btn btn-primary" onClick={restore}>
            <ArchiveRestore size={14} />
            Restore &amp; Open
          </button>
          <button
            className="btn btn-ghost btn-icon"
            onClick={closePreview}
            title="Close (Esc)"
            aria-label="Close"
          >
            <X size={16} />
          </button>
        </div>
      </header>

      <div ref={scrollRef} className="tp-body">
        {load.state === 'loading' && <Skeleton />}
        {load.state === 'error' && (
          <div className="tp-empty">
            <div>Couldn’t read this transcript.</div>
            <div className="tp-empty-detail selectable">{load.message}</div>
            <button className="btn" onClick={fetchTranscript}>
              Try again
            </button>
          </div>
        )}
        {load.state === 'ready' && load.messages.length === 0 && (
          <div className="tp-empty">No messages in this session.</div>
        )}
        {load.state === 'ready' && load.messages.length > 0 && (
          <ol className="tp-list">
            {load.messages.map((m, i) => (
              <Message
                key={i}
                index={i}
                message={m}
                highlight={terms}
                found={i === found}
                foundCall={i === found && find?.snippet?.kind === 'tool' ? find.snippet : undefined}
              />
            ))}
          </ol>
        )}
      </div>
    </>
  )
}

const COLLAPSE_LINES = 12
const COLLAPSE_CHARS = 1200

/**
 * The message a search led to: the turn the best match happened in, or the
 * first that says the words.
 */
function foundIndex(messages: TranscriptMessage[], find: PreviewFind, terms: string[]): number {
  const at = find.snippet?.timestamp
  if (at) {
    let turn = -1
    messages.forEach((m, i) => {
      if (m.timestamp > 0 && m.timestamp <= at) turn = i
    })
    if (turn >= 0) return turn
  }
  return messages.findIndex((m) => matchRanges(m.text, terms).length > 0)
}

/**
 * One message of a transcript. `author` names who said it, when that isn't
 * you and Claude. `highlight` marks a search's words, and `found` is the
 * message the search led to; `foundCall` is the tool call in it that matched,
 * which the transcript otherwise shows only by name.
 */
export function Message({
  message,
  author,
  index,
  highlight,
  found,
  foundCall
}: {
  message: TranscriptMessage
  author?: string
  index?: number
  highlight?: string[]
  found?: boolean
  foundCall?: SearchSnippet
}) {
  const text = message.text.trim()
  const long = text.split('\n').length > COLLAPSE_LINES || text.length > COLLAPSE_CHARS
  const [expanded, setExpanded] = useState(Boolean(found))
  const isUser = message.role === 'user'
  const ranges = useMemo(() => (highlight?.length ? matchRanges(text, highlight) : []), [text, highlight])

  return (
    <li
      className={found ? 'tp-msg is-found' : 'tp-msg'}
      data-index={index}
      data-role={message.role}
      data-notice={message.notice || undefined}
    >
      <div className="tp-msg-meta">
        <span className="tp-msg-author">{author ?? (isUser ? 'You' : 'Claude')}</span>
        {message.timestamp > 0 && <time>{formatTime(message.timestamp)}</time>}
      </div>
      {text && (
        <div className="tp-msg-content">
          <div className={long && !expanded ? 'tp-text is-collapsed' : 'tp-text'}>
            <div className="selectable">
              <Highlight text={text} ranges={ranges} />
            </div>
          </div>
          {long && (
            <button className="tp-more" onClick={() => setExpanded((v) => !v)}>
              {expanded ? 'Show less' : 'Show more'}
            </button>
          )}
        </div>
      )}
      {foundCall && (
        <div className="tp-found-call">
          <span className="tp-call-name">
            {foundCall.agent ? `Agent · ${foundCall.tool}` : foundCall.tool}
            {foundCall.file && <span className="tp-found-file">{foundCall.file}</span>}
          </span>
          <span className="tp-call-detail selectable">
            <Highlight text={foundCall.text} ranges={foundCall.ranges} />
          </span>
        </div>
      )}
      {message.calls ? (
        <ToolCalls calls={message.calls} />
      ) : message.tools && message.tools.length > 0 && (
        <div className="tp-tools">
          {message.tools.map((tool, i) => (
            <span key={i} className="tp-tool">
              {tool}
            </span>
          ))}
        </div>
      )}
    </li>
  )
}

/** Calls shown from a long run until the rest are asked for. */
const CALLS_SHOWN = 8

/** Calls an agent made, oldest first, each with what it acted on. Long runs show their latest calls. */
function ToolCalls({ calls }: { calls: ToolCall[] }) {
  const [all, setAll] = useState(false)
  const hidden = all ? 0 : Math.max(0, calls.length - CALLS_SHOWN)
  return (
    <div className="tp-calls">
      {hidden > 0 && (
        <button className="tp-more" onClick={() => setAll(true)}>
          Show {hidden} earlier {hidden === 1 ? 'call' : 'calls'}
        </button>
      )}
      {calls.slice(hidden).map((call, i) => (
        <div key={hidden + i} className="tp-call">
          <span className="tp-call-name">{call.name}</span>
          {call.detail && <span className="tp-call-detail selectable">{call.detail}</span>}
        </div>
      ))}
    </div>
  )
}

export function Skeleton() {
  return (
    <div className="tp-skeleton" aria-label="Loading transcript">
      {[72, 94, 58, 86, 40].map((w, i) => (
        <div key={i} className="tp-skel-row">
          <div className="tp-skel-line is-short" />
          <div className="tp-skel-line" style={{ width: `${w}%` }} />
        </div>
      ))}
    </div>
  )
}

/**
 * First spot to the right of the archive, filling columns top to bottom,
 * that doesn't overlap an active card.
 */
function freeSlotBesideArchive(board: BoardSnapshot, sessionId: string) {
  // Clear of the archive panel at its largest: it may be open behind this preview.
  const archive = stackZone(board.archive, STACK_ROWS * 2)
  const others = board.cards.filter((c) => !c.archived && c.sessionId !== sessionId)
  const rows = Math.max(3, Math.floor(archive.h / (CARD_H + GAP)))
  const overlaps = (x: number, y: number) =>
    others.some(
      (c) => x < c.x + CARD_W + GAP && x + CARD_W + GAP > c.x && y < c.y + CARD_H + GAP && y + CARD_H + GAP > c.y
    )

  for (let i = 0; i < rows * 40; i++) {
    const x = archive.x + archive.w + 80 + Math.floor(i / rows) * (CARD_W + GAP)
    const y = archive.y + (i % rows) * (CARD_H + GAP)
    if (!overlaps(x, y)) return { x, y }
  }
  return { x: archive.x + archive.w + 80, y: archive.y + others.length * 24 }
}

const dayFmt = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' })
const yearFmt = new Intl.DateTimeFormat(undefined, {
  month: 'short',
  day: 'numeric',
  year: 'numeric'
})
const timeFmt = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' })

function formatDay(ts: number) {
  const sameYear = new Date(ts).getFullYear() === new Date().getFullYear()
  return (sameYear ? dayFmt : yearFmt).format(ts)
}

function formatRange(start: number, end: number) {
  if (new Date(start).toDateString() === new Date(end).toDateString()) {
    return `${formatDay(start)}, ${timeFmt.format(start)} – ${timeFmt.format(end)}`
  }
  return `${formatDay(start)} – ${formatDay(end)}`
}

export function formatTime(ts: number) {
  const isToday = new Date(ts).toDateString() === new Date().toDateString()
  return isToday ? timeFmt.format(ts) : `${formatDay(ts)}, ${timeFmt.format(ts)}`
}
