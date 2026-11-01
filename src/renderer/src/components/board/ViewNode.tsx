import { memo, useCallback, useContext, useEffect, useRef, useState, type KeyboardEvent, type MouseEvent } from 'react'
import { NodeResizer, type Node, type NodeProps, type OnResizeEnd } from '@xyflow/react'
import {
  LayoutDashboard,
  LoaderCircle,
  Maximize2,
  MoreHorizontal,
  PencilLine,
  RefreshCw,
  SquareTerminal,
  Wand2,
  X
} from 'lucide-react'
import clsx from 'clsx'
import type { BoardView, ViewCommand } from '@shared/types'
import { Modal } from '@renderer/components/Modal'
import { useApp } from '@renderer/store'
import { RenamingContext, useBoardActions } from './BoardContext'
import { VIEW_MIN_H, VIEW_MIN_W } from './views'
import { registerFrame, unregisterFrame } from './viewRelay'
import './views.css'

export type ViewNodeData = { view: BoardView }
export type ViewFlowNode = Node<ViewNodeData, 'view'>

const stop = (e: MouseEvent) => e.stopPropagation()

const isWaiting = (c: ViewCommand) => c.approved !== c.run && c.denied !== c.run

/** IPC errors arrive as "Error invoking remote method '…': Error: …". */
const reason = (err: unknown) =>
  (err instanceof Error ? err.message : String(err)).replace(/^Error invoking remote method '[^']+': (Error: )?/, '')

/** "every 10 minutes", "every hour", "every 2 hours". */
export function everyLabel(minutes: number): string {
  if (minutes % 60 !== 0) return minutes === 1 ? 'every minute' : `every ${minutes} minutes`
  const hours = minutes / 60
  if (hours % 24 === 0) return hours === 24 ? 'every day' : `every ${hours / 24} days`
  return hours === 1 ? 'every hour' : `every ${hours} hours`
}

function ViewNodeComponent({ id, data, selected, dragging }: NodeProps<ViewFlowNode>) {
  const { view } = data
  const projectId = useApp((s) => s.board?.project.id)
  const patchView = useApp((s) => s.patchView)
  const brain = useApp((s) => (view.sessionId ? s.board?.cards.find((c) => c.sessionId === view.sessionId) : undefined))
  const { openViewSession, setRenaming, viewMenu } = useBoardActions()
  const renaming = useContext(RenamingContext) === id
  const [asking, setAsking] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [resizing, setResizing] = useState(false)
  const [large, setLarge] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const status = brain?.live ? brain.status : 'done'
  const working = status === 'working'
  const waiting = brain?.live && status === 'waiting'
  const waitingCommands = (view.commands ?? []).filter(isWaiting)
  const canRefresh = Boolean(view.refresh || view.commands?.some((c) => c.approved === c.run))

  const viewRef = useRef(view)
  viewRef.current = view
  // Stable, so React Flow doesn't rebind the resize handles on every render.
  const onResizeEnd = useCallback<OnResizeEnd>(
    (_e, r) => {
      setResizing(false)
      patchView({
        id: viewRef.current.id,
        x: Math.round(r.x),
        y: Math.round(r.y),
        w: Math.round(r.width),
        h: Math.round(r.height)
      })
    },
    [patchView]
  )
  const onResizeStart = useCallback(() => setResizing(true), [])

  useEffect(() => {
    if (!error) return
    const t = window.setTimeout(() => setError(null), 6000)
    return () => window.clearTimeout(t)
  }, [error])

  const refresh = async () => {
    if (!projectId || refreshing) return
    setRefreshing(true)
    try {
      const results = await window.api.views.refresh(projectId, view.id)
      const failed = results.find((r) => r.error || r.code !== 0)
      if (failed) setError(`“${failed.name}” ${failed.error ? `failed: ${failed.error}` : `exited with code ${failed.code}`}`)
    } catch (err) {
      setError(reason(err))
    } finally {
      setRefreshing(false)
    }
  }

  const approve = (name: string, allow: boolean) => {
    if (projectId) window.api.views.approve(projectId, view.id, name, allow).catch((err) => setError(reason(err)))
  }

  const build = () => {
    if (projectId) window.api.views.ask(projectId, view.id, 'Build this view.').catch((err) => setError(reason(err)))
  }

  return (
    <>
      <div
        className={clsx(
          'view',
          selected && 'is-selected',
          dragging && 'is-dragging',
          resizing && 'is-resizing',
          waiting && 'is-waiting'
        )}
      >
        <header className="view-head" onDoubleClick={stop}>
          {working || waiting ? (
            <span
              className="status-dot is-small"
              data-status={status}
              title={waiting ? 'Claude needs you to carry on with this view' : 'Claude is working on this view'}
            />
          ) : (
            <LayoutDashboard className="view-icon" size={14} strokeWidth={2} />
          )}
          {renaming ? (
            <TitleEditor view={view} />
          ) : (
            <h3
              className="view-title"
              title={view.description}
              onDoubleClick={(e) => {
                e.stopPropagation()
                setRenaming(view.id)
              }}
            >
              {view.title}
            </h3>
          )}
          <div className="view-tools nodrag">
            {canRefresh && (
              <button
                className="card-tool"
                onClick={refresh}
                disabled={refreshing}
                title={view.refresh ? 'Refresh: run its commands, and ask its brain for fresh data' : 'Refresh: run its commands'}
                aria-label="Refresh"
              >
                <RefreshCw size={14} strokeWidth={2} className={clsx(refreshing && 'spin')} />
              </button>
            )}
            <button
              className="card-tool"
              onClick={() => setAsking((a) => !a)}
              aria-expanded={asking}
              title="Ask Claude to change this view"
              aria-label="Change view"
            >
              <PencilLine size={14} strokeWidth={2} />
            </button>
            <button
              className="card-tool"
              onClick={() => openViewSession(view.id)}
              title="Open its brain, the Claude session that builds this view"
              aria-label="Open its session"
            >
              <SquareTerminal size={14} strokeWidth={2} />
            </button>
            <button
              className="card-tool"
              onClick={() => setLarge(true)}
              disabled={view.version === 0}
              title="Open large"
              aria-label="Open large"
            >
              <Maximize2 size={14} strokeWidth={2} />
            </button>
            <button
              className="card-tool"
              onClick={(e) => {
                const r = e.currentTarget.getBoundingClientRect()
                viewMenu(view.id, r.left, r.bottom + 4)
              }}
              title="More"
              aria-label="More"
            >
              <MoreHorizontal size={14} strokeWidth={2} />
            </button>
          </div>
        </header>

        {waitingCommands.length > 0 && (
          <div className="view-approvals nodrag">
            {waitingCommands.map((c) => (
              <div key={c.name} className="view-approve">
                <p className="view-approve-text">
                  This view wants to run{c.every ? ` ${everyLabel(c.every)}` : ''}:
                  <code className="view-approve-cmd">{c.run}</code>
                </p>
                <div className="view-approve-actions">
                  <button className="btn btn-ghost" onClick={() => approve(c.name, false)}>
                    Don’t allow
                  </button>
                  <button className="btn btn-primary" onClick={() => approve(c.name, true)}>
                    Allow
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}

        <div className="view-body">
          {view.version > 0 && projectId ? (
            <>
              <ViewFrame projectId={projectId} view={view} />
              {/* Until it's selected, the page is a picture: clicks select it and scrolling pans the board. */}
              {!selected && (
                <div
                  className="view-shield"
                  onDoubleClick={(e) => {
                    e.stopPropagation()
                    setLarge(true)
                  }}
                >
                  <span className="view-shield-hint">Click to use</span>
                </div>
              )}
            </>
          ) : (
            <div className="view-empty">
              {working ? (
                <>
                  <LoaderCircle size={18} className="spin" />
                  <p className="view-empty-title">Claude is building this view…</p>
                  <button className="btn btn-ghost nodrag" onClick={() => openViewSession(view.id)}>
                    Watch it work
                  </button>
                </>
              ) : waiting ? (
                <>
                  <p className="view-empty-title">Claude needs you to carry on</p>
                  <button className="btn nodrag" onClick={() => openViewSession(view.id)}>
                    Open its session
                  </button>
                </>
              ) : (
                <>
                  <p className="view-empty-title">No page yet</p>
                  <button className="btn nodrag" onClick={build}>
                    <Wand2 size={14} strokeWidth={2} />
                    Build it
                  </button>
                </>
              )}
              <p className="view-empty-description">{view.description}</p>
            </div>
          )}
        </div>

        {error && (
          <p className="view-error" role="status">
            {error}
          </p>
        )}
        {asking && projectId && (
          <AskBar
            projectId={projectId}
            view={view}
            onDone={() => setAsking(false)}
            onError={(message) => setError(message)}
          />
        )}
      </div>
      <NodeResizer
        isVisible={selected && !renaming}
        minWidth={VIEW_MIN_W}
        minHeight={VIEW_MIN_H}
        lineClassName="note-resize-line"
        handleClassName="note-resize-handle"
        onResizeStart={onResizeStart}
        onResizeEnd={onResizeEnd}
      />
      {large && projectId && <ViewModal projectId={projectId} view={view} onClose={() => setLarge(false)} />}
    </>
  )
}

export const ViewNode = memo(ViewNodeComponent)

/** The view's page, in a sandboxed frame of its own origin with nothing but scripts allowed. */
export function ViewFrame({ projectId, view }: { projectId: string; view: BoardView }) {
  const ref = useRef<HTMLIFrameElement>(null)
  useEffect(() => {
    const win = ref.current?.contentWindow
    if (!win) return
    registerFrame(win, { projectId, viewId: view.id, version: view.version })
    return () => unregisterFrame(win)
  }, [projectId, view.id, view.version])

  return (
    <iframe
      ref={ref}
      className="view-frame"
      title={view.title}
      sandbox="allow-scripts"
      src={`nodus-view://${view.id}/?v=${view.version}`}
    />
  )
}

/** The view, large, over the board. */
function ViewModal({ projectId, view, onClose }: { projectId: string; view: BoardView; onClose: () => void }) {
  return (
    <Modal open onClose={onClose} className="view-modal" labelledBy={`view-modal-${view.id}`}>
      <header className="view-modal-head">
        <LayoutDashboard className="view-icon" size={15} strokeWidth={2} />
        <h2 id={`view-modal-${view.id}`} className="view-modal-title">
          {view.title}
        </h2>
        <button className="card-tool" onClick={onClose} title="Close" aria-label="Close">
          <X size={15} strokeWidth={2} />
        </button>
      </header>
      <div className="view-modal-body">
        <ViewFrame projectId={projectId} view={view} />
      </div>
    </Modal>
  )
}

/** Tells the view's brain what to change. */
function AskBar({
  projectId,
  view,
  onDone,
  onError
}: {
  projectId: string
  view: BoardView
  onDone: () => void
  onError: (message: string) => void
}) {
  const [value, setValue] = useState('')
  const [sending, setSending] = useState(false)
  const ref = useRef<HTMLInputElement>(null)
  useEffect(() => ref.current?.focus({ preventScroll: true }), [])

  const send = async () => {
    const prompt = value.trim()
    if (!prompt || sending) return
    setSending(true)
    try {
      await window.api.views.ask(projectId, view.id, prompt)
      onDone()
    } catch (err) {
      onError(reason(err))
      setSending(false)
    }
  }

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    e.stopPropagation()
    if (e.key === 'Enter') void send()
    if (e.key === 'Escape') onDone()
  }

  return (
    <div className="view-ask nodrag nopan">
      <input
        ref={ref}
        className="view-ask-input nowheel"
        value={value}
        placeholder="Tell Claude what to change…"
        disabled={sending}
        spellCheck={false}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={onKeyDown}
        onPointerDown={(e) => e.stopPropagation()}
        onDoubleClick={stop}
      />
      <button className="btn btn-primary" onClick={() => void send()} disabled={!value.trim() || sending}>
        {sending ? <LoaderCircle size={14} className="spin" /> : 'Send'}
      </button>
    </div>
  )
}

function TitleEditor({ view }: { view: BoardView }) {
  const { renameView, setRenaming } = useBoardActions()
  const [value, setValue] = useState(view.title)
  const ref = useRef<HTMLInputElement>(null)
  const done = useRef(false)

  useEffect(() => {
    ref.current?.focus()
    ref.current?.select()
  }, [])

  const commit = () => {
    if (done.current) return
    done.current = true
    if (value.trim() && value.trim() !== view.title) renameView(view.id, value.trim())
    setRenaming(null)
  }

  return (
    <input
      ref={ref}
      className="view-title-input nodrag nopan"
      value={value}
      placeholder="View name"
      spellCheck={false}
      onChange={(e) => setValue(e.target.value)}
      onBlur={commit}
      onPointerDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation()
        if (e.key === 'Enter') commit()
        if (e.key === 'Escape') {
          done.current = true
          setRenaming(null)
        }
      }}
    />
  )
}
