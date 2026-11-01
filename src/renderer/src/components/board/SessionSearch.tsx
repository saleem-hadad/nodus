import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { Archive, Check, Loader2, Search, Sparkles } from 'lucide-react'
import clsx from 'clsx'
import type { SearchSnippet, SessionCard, SessionSearchHit } from '@shared/types'
import { matchRanges, queryTerms } from '@shared/search'
import { Highlight } from '@renderer/components/Highlight'
import { Modal } from '@renderer/components/Modal'
import { cardTitle } from '@renderer/store'
import { timeAgo } from './layout'
import './search.css'

const DEBOUNCE_MS = 120
const RECENT = 30

const STATUS_LABEL = { working: 'Working', waiting: 'Needs you', done: 'Done' } as const

interface Props {
  projectId: string
  /** The board's Claude sessions, live and archived. */
  cards: SessionCard[]
  query: string
  onQueryChange: (query: string) => void
  onClose: () => void
  /** Shows the session's card on the board, or an archived one's transcript at its best match. */
  onJump: (sessionId: string, best?: SearchSnippet) => void
  /** Starts a session from what these sessions did about the query. */
  onStart: (sessionIds: string[]) => void
}

interface Row {
  card: SessionCard
  hit?: SessionSearchHit
}

/**
 * ⌘K: searches what was said and done in every session on the board, live
 * and archived. With nothing typed it lists the latest sessions, so it also
 * gets you to a card by name.
 */
export function SessionSearch({ projectId, cards, query, onQueryChange, onClose, onJump, onStart }: Props) {
  const [hits, setHits] = useState<{ query: string; hits: SessionSearchHit[] } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [active, setActive] = useState(0)
  const [marked, setMarked] = useState<string[]>([])
  const listRef = useRef<HTMLDivElement>(null)
  const trimmed = query.trim()

  useEffect(() => {
    if (!trimmed) return
    let stale = false
    const t = window.setTimeout(() => {
      window.api.sessions.search(projectId, trimmed).then(
        (found) => {
          if (stale) return
          setHits({ query: trimmed, hits: found })
          setError(null)
        },
        (err: unknown) => !stale && setError(err instanceof Error ? err.message : String(err))
      )
    }, DEBOUNCE_MS)
    return () => {
      stale = true
      window.clearTimeout(t)
    }
  }, [projectId, trimmed])

  const byId = useMemo(() => new Map(cards.map((c) => [c.sessionId, c])), [cards])
  const rows = useMemo<Row[]>(() => {
    if (!trimmed) {
      return [...cards]
        .sort((a, b) => Number(a.archived) - Number(b.archived) || updatedAt(b) - updatedAt(a))
        .slice(0, RECENT)
        .map((card) => ({ card }))
    }
    // Until the first answer comes, nothing; after that, the last answer stays while the next is on its way.
    return (hits?.hits ?? []).flatMap((hit) => {
      const card = byId.get(hit.sessionId)
      return card ? [{ card, hit }] : []
    })
  }, [trimmed, cards, hits, byId])
  const loading = Boolean(trimmed) && hits?.query !== trimmed && !error
  const terms = useMemo(() => queryTerms(hits?.query ?? ''), [hits])

  // New results start at the top. The latest sessions keep their place as cards update.
  useEffect(() => setActive(0), [trimmed, hits])
  const current = Math.max(0, Math.min(active, rows.length - 1))

  useEffect(() => {
    listRef.current?.querySelector(`[data-index="${current}"]`)?.scrollIntoView({ block: 'nearest' })
  }, [current])

  const jump = (row: Row) => onJump(row.card.sessionId, row.hit?.snippets[0])

  const toggle = (sessionId: string) =>
    setMarked((m) => (m.includes(sessionId) ? m.filter((id) => id !== sessionId) : [...m, sessionId]))

  const start = () => {
    const ids = marked.length ? marked : rows[current] ? [rows[current].card.sessionId] : []
    if (ids.length) onStart(ids)
  }

  const onKeyDown = (e: KeyboardEvent) => {
    const last = rows.length - 1
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setActive((i) => Math.min(last, i + 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setActive((i) => Math.max(0, i - 1))
    } else if (e.key === 'Enter' && e.metaKey) {
      e.preventDefault()
      start()
    } else if (e.key === 'Enter') {
      e.preventDefault()
      if (rows[current]) jump(rows[current])
    } else if (e.key === 'Tab') {
      // Like fzf: Tab marks and moves on.
      e.preventDefault()
      if (!rows[current]) return
      toggle(rows[current].card.sessionId)
      setActive((i) => (e.shiftKey ? Math.max(0, i - 1) : Math.min(last, i + 1)))
    } else if (e.key.toLowerCase() === 'k' && e.metaKey) {
      e.preventDefault()
      onClose()
    }
  }

  const empty = trimmed && !loading && hits?.query === trimmed && rows.length === 0

  return (
    <Modal open onClose={onClose} className="search-dialog" labelledBy="search-input">
      <div className="search-inner" onKeyDown={onKeyDown}>
        <div className="search-field">
          {loading ? (
            <Loader2 size={16} className="search-field-icon spin" />
          ) : (
            <Search size={16} strokeWidth={2.2} className="search-field-icon" />
          )}
          <input
            id="search-input"
            className="search-input"
            value={query}
            autoFocus
            spellCheck={false}
            placeholder="Search what every session said and did"
            aria-label="Search sessions"
            aria-controls="search-results"
            aria-activedescendant={rows[current] ? `search-row-${current}` : undefined}
            onFocus={(e) => e.currentTarget.select()}
            onChange={(e) => onQueryChange(e.target.value)}
          />
          <kbd className="search-esc">esc</kbd>
        </div>

        <div className="search-results" id="search-results" role="listbox" ref={listRef} aria-multiselectable>
          {!trimmed && rows.length > 0 && <div className="search-section">Latest sessions</div>}
          {rows.map((row, i) => (
            <ResultRow
              key={row.card.sessionId}
              index={i}
              row={row}
              terms={terms}
              active={i === current}
              marked={marked.includes(row.card.sessionId)}
              onHover={() => setActive(i)}
              onOpen={() => jump(row)}
              onToggle={() => toggle(row.card.sessionId)}
            />
          ))}
          {empty && (
            <div className="search-empty">
              No session on this board mentions <strong>{trimmed}</strong>.
              <span>Prompts, replies, file paths, commands and edits are searched; tool output isn’t.</span>
            </div>
          )}
          {error && <div className="search-empty search-error">{error}</div>}
          {!trimmed && rows.length === 0 && <div className="search-empty">No sessions on this board yet.</div>}
        </div>

        <footer className="search-foot">
          <span>
            <kbd>↑</kbd>
            <kbd>↓</kbd> move
          </span>
          <span>
            <kbd>↩</kbd> {rows[current]?.card.archived ? 'read it' : 'go to card'}
          </span>
          <span>
            <kbd>⇥</kbd> select
          </span>
          <button
            className={clsx('search-start', marked.length > 0 && 'is-ready')}
            disabled={!rows.length && !marked.length}
            onClick={start}
          >
            <Sparkles size={13} strokeWidth={2.2} />
            {marked.length > 1
              ? `New session from ${marked.length}`
              : marked.length === 1
                ? 'New session from 1'
                : 'New session from this'}
            <kbd>⌘</kbd>
            <kbd>↩</kbd>
          </button>
        </footer>
      </div>
    </Modal>
  )
}

function updatedAt(card: SessionCard) {
  return card.meta?.updatedAt ?? card.createdAt
}

function ResultRow({
  index,
  row,
  terms,
  active,
  marked,
  onHover,
  onOpen,
  onToggle
}: {
  index: number
  row: Row
  terms: string[]
  active: boolean
  marked: boolean
  onHover: () => void
  onOpen: () => void
  onToggle: () => void
}) {
  const { card, hit } = row
  const title = cardTitle(card)
  const status = card.archived ? null : card.status
  return (
    <div
      id={`search-row-${index}`}
      data-index={index}
      role="option"
      aria-selected={marked}
      className={clsx('search-row', active && 'is-active', marked && 'is-marked')}
      onMouseMove={onHover}
      onClick={(e) => (e.metaKey ? onToggle() : onOpen())}
    >
      <button
        className="search-check"
        tabIndex={-1}
        aria-label={marked ? `Unselect ${title}` : `Select ${title}`}
        onClick={(e) => {
          e.stopPropagation()
          onToggle()
        }}
      >
        {marked && <Check size={11} strokeWidth={3} />}
      </button>
      <div className="search-row-body">
        <div className="search-row-head">
          {status ? (
            <span className="status-dot is-small" data-status={status} title={STATUS_LABEL[status]} />
          ) : (
            <Archive size={12} strokeWidth={2.2} className="search-archived" aria-label="Archived" />
          )}
          <span className="search-title">
            <Highlight text={title} ranges={matchRanges(title, terms)} />
          </span>
          {hit && hit.matches > 0 && (
            <span className="search-count">
              {hit.matches} {hit.matches === 1 ? 'match' : 'matches'}
            </span>
          )}
          <span className="search-time">{timeAgo(updatedAt(card))}</span>
        </div>
        {hit?.snippets.map((s, i) => <SnippetLine key={i} snippet={s} />)}
        {!hit && card.meta?.lastPrompt && <div className="search-snippet is-plain">{card.meta.lastPrompt}</div>}
      </div>
    </div>
  )
}

function SnippetLine({ snippet: s }: { snippet: SearchSnippet }) {
  const who = s.kind === 'prompt' ? 'You' : s.kind === 'reply' ? 'Claude' : s.tool
  return (
    <div className="search-snippet">
      <span className="search-kind" data-kind={s.kind}>
        {s.agent && <span className="search-agent">Agent</span>}
        {who}
        {s.file && <span className="search-file">{s.file}</span>}
      </span>
      <span className={clsx('search-text', s.kind === 'tool' && 'is-code')}>
        <Highlight text={s.text} ranges={s.ranges} />
      </span>
    </div>
  )
}
