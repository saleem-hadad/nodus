import {
  forwardRef,
  memo,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type RefObject
} from 'react'
import { ViewportPortal, useReactFlow } from '@xyflow/react'
import { Eraser, MousePointer2, MoveUpRight, Pencil, Trash2, Type, type LucideIcon } from 'lucide-react'
import clsx from 'clsx'
import { INK_COLORS, type InkColor, type InkStroke } from '@shared/types'
import { useApp } from '@renderer/store'
import { keys } from '@renderer/platform'
import {
  ARROW_HEAD,
  LABEL_FONT,
  LABEL_LINE,
  PEN_WIDTH,
  arrowPath,
  distance,
  flatten,
  penPath,
  pointsOf,
  simplify,
  strokeAt,
  strokeBox,
  strokesAlong,
  translate,
  type Pt
} from './ink'
import './ink.css'

export type InkTool = 'select' | 'pen' | 'arrow' | 'text' | 'eraser'

/** The board's single-key shortcuts for the tools (Esc goes back to the pointer). */
export const INK_SHORTCUTS: Record<string, InkTool> = { p: 'pen', a: 'arrow', x: 'eraser' }

export interface InkHandle {
  setTool(tool: InkTool): void
  /** Esc: back to the pointer, or lets go of the selected stroke. False when there was nothing to do. */
  escape(): boolean
  /** ⌫: deletes the selected stroke. False when none is selected. */
  deleteSelected(): boolean
  undo(): boolean
  redo(): boolean
}

interface Props {
  wrapperRef: RefObject<HTMLDivElement | null>
  /** Lets go of the selected cards and notes, when a stroke is selected instead. */
  onSelect: () => void
  onToast: (message: string) => void
}

const TOOLS: { tool: InkTool; label: string; key?: string; Icon: LucideIcon }[] = [
  { tool: 'select', label: 'Pointer', key: 'Esc', Icon: MousePointer2 },
  { tool: 'pen', label: 'Pen', key: 'P', Icon: Pencil },
  { tool: 'arrow', label: 'Arrow', key: 'A', Icon: MoveUpRight },
  { tool: 'text', label: 'Text', Icon: Type },
  { tool: 'eraser', label: 'Eraser', key: 'X', Icon: Eraser }
]
const COLOR_NAMES: Record<InkColor, string> = {
  graphite: 'Graphite',
  red: 'Red',
  orange: 'Orange',
  green: 'Green',
  blue: 'Blue',
  purple: 'Purple'
}
const COLOR_KEY = 'nodus.inkColor'
const EMPTY: InkStroke[] = []
/** How many changes ⌘Z can take back. */
const HISTORY = 100
/** How close to a stroke the pointer counts as on it, in screen pixels. */
const HIT_PX = 6
/** Points closer together than this on screen add nothing to a line. */
const MIN_STEP_PX = 1.5
/** An arrow shorter than this on screen is a slip, not an arrow. */
const MIN_ARROW_PX = 8

/** Ink keeps the thickness it had on screen when it was drawn. */
const sizeAt = (zoom: number) => Math.round(Math.min(4, Math.max(0.5, 1 / zoom)) * 100) / 100

const ink = (color: InkColor) => `var(--ink-${color})`

function readColor(): InkColor {
  try {
    const c = localStorage.getItem(COLOR_KEY) as InkColor | null
    if (c && INK_COLORS.includes(c)) return c
  } catch {
    // No storage: start with red.
  }
  return 'red'
}

/** A label being typed: a new one, or one already on the board (`id`). */
interface LabelDraft {
  key: number
  id?: string
  x: number
  y: number
  size: number
  color: InkColor
  text: string
}

/**
 * Ink on the board: pen lines, arrows and text labels drawn over everything,
 * and the dock of tools that draws them, above the zoom controls. While a
 * tool is out, the board's own pointer handling (dragging cards, the selection
 * box, double-click for a new session) stands aside; the pointer tool selects,
 * moves and deletes ink.
 */
export const InkLayer = memo(
  forwardRef<InkHandle, Props>(function InkLayer({ wrapperRef, onSelect, onToast }, ref) {
    const rf = useReactFlow()
    const strokes = useApp((s) => s.board?.ink ?? EMPTY)
    const projectId = useApp((s) => s.board?.project.id)
    const saveInk = useApp((s) => s.saveInk)
    const strokesRef = useRef(strokes)
    strokesRef.current = strokes

    const [tool, setToolState] = useState<InkTool>('select')
    const toolRef = useRef(tool)
    const [color, setColorState] = useState<InkColor>(readColor)
    const colorRef = useRef(color)
    const [selected, setSelectedState] = useState<string | null>(null)
    const selectedRef = useRef(selected)
    const [live, setLive] = useState<InkStroke | null>(null)
    const [moving, setMoving] = useState<{ id: string; dx: number; dy: number } | null>(null)
    const [erasing, setErasing] = useState<ReadonlySet<string> | null>(null)
    const [label, setLabelState] = useState<LabelDraft | null>(null)
    const labelRef = useRef(label)
    const [paletteOpen, setPaletteOpen] = useState(false)

    const past = useRef<InkStroke[][]>([])
    const future = useRef<InkStroke[][]>([])
    const swallowClick = useRef(false)
    const spaceDown = useRef(false)
    const onSelectRef = useRef(onSelect)
    onSelectRef.current = onSelect

    const select = useCallback((id: string | null) => {
      selectedRef.current = id
      setSelectedState(id)
    }, [])
    const setLabel = useCallback((next: LabelDraft | null) => {
      labelRef.current = next
      setLabelState(next)
    }, [])

    /** Saves a change to the ink, so ⌘Z can take it back. */
    const commit = useCallback(
      (next: InkStroke[]) => {
        past.current.push(strokesRef.current)
        if (past.current.length > HISTORY) past.current.shift()
        future.current = []
        strokesRef.current = next
        saveInk(next)
      },
      [saveInk]
    )

    // Another board has its own history.
    useEffect(() => {
      past.current = []
      future.current = []
      select(null)
      setLabel(null)
    }, [projectId, select, setLabel])

    /** Puts a label being typed on the board (or takes it off, emptied). */
    const finishLabel = useCallback(() => {
      const draft = labelRef.current
      if (!draft) return
      setLabel(null)
      const text = draft.text.trim()
      const current = strokesRef.current
      if (draft.id) {
        const old = current.find((s) => s.id === draft.id)
        if (!old || old.text === text) return
        commit(text ? current.map((s) => (s.id === draft.id ? { ...s, text } : s)) : current.filter((s) => s.id !== draft.id))
      } else if (text) {
        commit([
          ...current,
          { id: crypto.randomUUID(), kind: 'text', color: draft.color, points: flatten([[draft.x, draft.y]]), size: draft.size, text }
        ])
      }
    }, [commit, setLabel])

    const setTool = useCallback(
      (next: InkTool) => {
        finishLabel()
        toolRef.current = next
        setToolState(next)
        if (next !== 'select') select(null)
      },
      [finishLabel, select]
    )

    const pickColor = useCallback(
      (next: InkColor) => {
        colorRef.current = next
        setColorState(next)
        setPaletteOpen(false)
        try {
          localStorage.setItem(COLOR_KEY, next)
        } catch {
          // Not remembered, then.
        }
        // A selected stroke takes the colour too.
        const id = selectedRef.current
        const current = strokesRef.current
        if (id && current.some((s) => s.id === id && s.color !== next)) {
          commit(current.map((s) => (s.id === id ? { ...s, color: next } : s)))
        }
      },
      [commit]
    )

    const clearAll = useCallback(() => {
      finishLabel()
      if (strokesRef.current.length === 0) return
      select(null)
      commit([])
      onToast(`Cleared the ink. Press ${keys('⌘Z')} to bring it back.`)
    }, [commit, finishLabel, onToast, select])

    useImperativeHandle(
      ref,
      () => ({
        setTool,
        escape() {
          if (toolRef.current !== 'select') {
            setTool('select')
            return true
          }
          if (selectedRef.current) {
            select(null)
            return true
          }
          return false
        },
        deleteSelected() {
          const id = selectedRef.current
          if (!id) return false
          select(null)
          commit(strokesRef.current.filter((s) => s.id !== id))
          return true
        },
        undo() {
          finishLabel()
          const prev = past.current.pop()
          if (!prev) return false
          future.current.push(strokesRef.current)
          strokesRef.current = prev
          select(null)
          saveInk(prev)
          return true
        },
        redo() {
          const next = future.current.pop()
          if (!next) return false
          past.current.push(strokesRef.current)
          strokesRef.current = next
          select(null)
          saveInk(next)
          return true
        }
      }),
      [commit, finishLabel, saveInk, select, setTool]
    )

    // The cursor for the tool that's out (ink.css).
    useEffect(() => {
      const el = wrapperRef.current
      if (!el) return
      el.dataset.inkTool = tool
      if (tool !== 'select') el.classList.remove('ink-hover')
    }, [tool, wrapperRef])

    // ---- pointer ------------------------------------------------------------
    // Listened for on the window in the capture phase, ahead of React Flow (whose
    // pane starts its selection box from a capture handler of its own), so a
    // gesture the ink takes never reaches the board underneath.

    useEffect(() => {
      const wrapper = wrapperRef.current
      if (!wrapper) return

      const toFlow = (e: { clientX: number; clientY: number }): Pt => {
        const p = rf.screenToFlowPosition({ x: e.clientX, y: e.clientY })
        return [p.x, p.y]
      }
      const zoom = () => rf.getViewport().zoom
      /** Whether the event is on the board itself, not its controls, rails or bars. */
      const onBoard = (target: EventTarget | null): target is Element =>
        target instanceof Element && wrapper.contains(target) && Boolean(target.closest('.react-flow__renderer'))

      /** Takes the gesture from the board: no card drag, selection box or click under it. */
      const take = (e: PointerEvent) => {
        e.stopPropagation()
        e.preventDefault()
        swallowClick.current = true
        finishLabel()
        // A note being written or a terminal keeps the keyboard otherwise, and with it Esc.
        const active = document.activeElement
        if (active instanceof HTMLElement && wrapper.contains(active)) active.blur()
        try {
          wrapper.setPointerCapture(e.pointerId)
        } catch {
          // The pointer already went up.
        }
      }

      /** Follows the pointer until it lifts. */
      const follow = (e: PointerEvent, onMove: (ev: PointerEvent) => void, onEnd: (cancelled: boolean) => void) => {
        const id = e.pointerId
        const move = (ev: PointerEvent) => {
          if (ev.pointerId === id) onMove(ev)
        }
        const end = (ev: PointerEvent) => {
          if (ev.pointerId !== id) return
          window.removeEventListener('pointermove', move)
          window.removeEventListener('pointerup', end)
          window.removeEventListener('pointercancel', end)
          onEnd(ev.type === 'pointercancel')
        }
        window.addEventListener('pointermove', move)
        window.addEventListener('pointerup', end)
        window.addEventListener('pointercancel', end)
      }

      const draw = (e: PointerEvent, kind: 'pen' | 'arrow', start: Pt) => {
        const z = zoom()
        const base: InkStroke = { id: crypto.randomUUID(), kind, color: colorRef.current, size: sizeAt(z), points: [] }
        const pts: Pt[] = [start]
        setLive({ ...base, points: flatten(pts) })
        follow(
          e,
          (ev) => {
            const events = kind === 'pen' ? ev.getCoalescedEvents?.() : undefined
            for (const one of events?.length ? events : [ev]) {
              const q = toFlow(one)
              if (kind === 'arrow') pts[1] = q
              else if (distance(q, pts[pts.length - 1]) * z >= MIN_STEP_PX) pts.push(q)
            }
            setLive({ ...base, points: flatten(pts) })
          },
          (cancelled) => {
            setLive(null)
            if (cancelled) return
            if (kind === 'arrow' && (pts.length < 2 || distance(pts[0], pts[1]) * z < MIN_ARROW_PX)) return
            const kept = kind === 'pen' ? simplify(pts, 0.5 / z) : pts
            commit([...strokesRef.current, { ...base, points: flatten(kept) }])
          }
        )
      }

      const erase = (e: PointerEvent, start: Pt) => {
        const tolerance = HIT_PX / zoom()
        const gone = new Set<string>()
        let last = start
        const along = (a: Pt, b: Pt) => {
          const left = strokesRef.current.filter((s) => !gone.has(s.id))
          const hit = strokesAlong(left, a, b, tolerance)
          if (hit.size === 0) return
          for (const id of hit) gone.add(id)
          setErasing(new Set(gone))
        }
        along(start, start)
        follow(
          e,
          (ev) => {
            const q = toFlow(ev)
            along(last, q)
            last = q
          },
          (cancelled) => {
            setErasing(null)
            if (cancelled || gone.size === 0) return
            if (selectedRef.current && gone.has(selectedRef.current)) select(null)
            commit(strokesRef.current.filter((s) => !gone.has(s.id)))
          }
        )
      }

      const move = (e: PointerEvent, stroke: InkStroke, start: Pt) => {
        const z = zoom()
        let dx = 0
        let dy = 0
        let moved = false
        follow(
          e,
          (ev) => {
            const q = toFlow(ev)
            dx = q[0] - start[0]
            dy = q[1] - start[1]
            if (!moved && Math.hypot(dx, dy) * z < 3) return
            moved = true
            setMoving({ id: stroke.id, dx, dy })
          },
          (cancelled) => {
            setMoving(null)
            if (!moved || cancelled) return
            commit(strokesRef.current.map((s) => (s.id === stroke.id ? translate(s, dx, dy) : s)))
          }
        )
      }

      const placeLabel = (p: Pt) => {
        const size = sizeAt(zoom())
        // The click lands mid-line, where the caret shows.
        setLabel({ key: Date.now(), x: p[0], y: p[1] - (LABEL_LINE * size) / 2, size, color: colorRef.current, text: '' })
      }

      const editLabel = (s: InkStroke) => {
        const [x, y] = pointsOf(s)[0]
        select(null)
        setLabel({ key: Date.now(), id: s.id, x, y, size: s.size, color: s.color, text: s.text ?? '' })
      }

      const onPointerDown = (e: PointerEvent) => {
        swallowClick.current = false
        if (e.button !== 0 || !e.isPrimary || spaceDown.current || !onBoard(e.target)) return
        if (e.target.closest('.ink-label-input')) return
        const p = toFlow(e)
        const current = toolRef.current
        if (current === 'select') {
          const hit = strokeAt(strokesRef.current, p, HIT_PX / zoom())
          if (!hit) {
            if (selectedRef.current) select(null)
            return
          }
          take(e)
          select(hit.id)
          onSelectRef.current()
          move(e, hit, p)
          return
        }
        take(e)
        if (current === 'pen' || current === 'arrow') draw(e, current, p)
        else if (current === 'eraser') erase(e, p)
        else placeLabel(p)
      }

      const onClick = (e: MouseEvent) => {
        if (!swallowClick.current) return
        swallowClick.current = false
        e.stopPropagation()
        e.preventDefault()
      }

      const onDoubleClick = (e: MouseEvent) => {
        // While the ink holds the pointer, its clicks land on the wrapper itself.
        if (e.target !== wrapper && !onBoard(e.target)) return
        if (e.target instanceof Element && e.target.closest('.ink-label-input')) return
        if (toolRef.current !== 'select') {
          // No new session from a double-click while drawing.
          e.stopPropagation()
          return
        }
        const hit = strokeAt(strokesRef.current, toFlow(e), HIT_PX / zoom())
        if (!hit) return
        e.stopPropagation()
        if (hit.kind === 'text') editLabel(hit)
      }

      // With the pointer, ink under it shows it can be picked up.
      let frame = 0
      const onHover = (e: PointerEvent) => {
        if (toolRef.current !== 'select' || e.buttons !== 0) return
        const target = e.target
        cancelAnimationFrame(frame)
        frame = requestAnimationFrame(() => {
          const over =
            strokesRef.current.length > 0 &&
            onBoard(target) &&
            Boolean(strokeAt(strokesRef.current, toFlow(e), HIT_PX / zoom()))
          wrapper.classList.toggle('ink-hover', over)
        })
      }

      // Space held pans the board, whatever tool is out.
      const typing = (t: EventTarget | null) =>
        t instanceof Element && Boolean(t.closest('input, textarea, select, [contenteditable="true"]'))
      const onKeyDown = (e: KeyboardEvent) => {
        if (e.code === 'Space' && !typing(e.target)) spaceDown.current = true
      }
      const onKeyUp = (e: KeyboardEvent) => {
        if (e.code === 'Space') spaceDown.current = false
      }
      const onBlur = () => {
        spaceDown.current = false
      }

      window.addEventListener('pointerdown', onPointerDown, true)
      window.addEventListener('click', onClick, true)
      window.addEventListener('dblclick', onDoubleClick, true)
      window.addEventListener('keydown', onKeyDown, true)
      window.addEventListener('keyup', onKeyUp, true)
      window.addEventListener('blur', onBlur)
      wrapper.addEventListener('pointermove', onHover)
      return () => {
        cancelAnimationFrame(frame)
        window.removeEventListener('pointerdown', onPointerDown, true)
        window.removeEventListener('click', onClick, true)
        window.removeEventListener('dblclick', onDoubleClick, true)
        window.removeEventListener('keydown', onKeyDown, true)
        window.removeEventListener('keyup', onKeyUp, true)
        window.removeEventListener('blur', onBlur)
        wrapper.removeEventListener('pointermove', onHover)
        wrapper.classList.remove('ink-hover')
      }
    }, [rf, wrapperRef, commit, finishLabel, select, setLabel])

    // The colour palette closes on a click anywhere else.
    useEffect(() => {
      if (!paletteOpen) return
      const onDown = (e: PointerEvent) => {
        if (!(e.target instanceof Element) || !e.target.closest('.ink-color')) setPaletteOpen(false)
      }
      window.addEventListener('pointerdown', onDown, true)
      return () => window.removeEventListener('pointerdown', onDown, true)
    }, [paletteOpen])

    // ---- drawing --------------------------------------------------------------

    const shown = useMemo(
      () =>
        strokes.map((s) => {
          if (moving?.id === s.id) return translate(s, moving.dx, moving.dy)
          return s
        }),
      [strokes, moving]
    )
    const lines = shown.filter((s) => s.kind !== 'text')
    const labels = shown.filter((s) => s.kind === 'text' && s.id !== label?.id)
    const picked = selected ? shown.find((s) => s.id === selected) : undefined
    const box = picked && strokeBox(picked)
    const boxPad = 6 * (picked?.size ?? 1)

    return (
      <>
        <ViewportPortal>
          <div className="ink-layer">
            <svg className="ink-svg" width="1" height="1" aria-hidden>
              {lines.map((s) => (
                <InkLine key={s.id} stroke={s} erasing={Boolean(erasing?.has(s.id))} />
              ))}
              {live && <InkLine stroke={live} erasing={false} />}
              {box && (
                <rect
                  className="ink-selection"
                  x={box.x - boxPad}
                  y={box.y - boxPad}
                  width={box.w + 2 * boxPad}
                  height={box.h + 2 * boxPad}
                  rx={4 * (picked?.size ?? 1)}
                />
              )}
            </svg>
            {labels.map((s) => (
              <div
                key={s.id}
                className={clsx('ink-label', erasing?.has(s.id) && 'is-erasing')}
                style={labelStyle(s.points[0], s.points[1], s.size, s.color)}
              >
                {s.text}
              </div>
            ))}
            {label && (
              <input
                key={label.key}
                className="ink-label-input nodrag nopan"
                style={labelStyle(label.x, label.y, label.size, label.color)}
                value={label.text}
                placeholder="Label"
                aria-label="Label"
                autoFocus
                spellCheck={false}
                onChange={(e) => setLabel({ ...label, text: e.target.value })}
                onBlur={finishLabel}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    e.preventDefault()
                    finishLabel()
                  } else if (e.key === 'Escape') {
                    // Leaves a new label off the board, and an edited one as it was.
                    e.preventDefault()
                    e.stopPropagation()
                    setLabel(null)
                  }
                }}
              />
            )}
          </div>
        </ViewportPortal>

        <div className="ink-dock" role="toolbar" aria-label="Ink" onPointerDown={(e) => e.preventDefault()}>
          {TOOLS.map(({ tool: t, label: name, key, Icon }) => (
            <button
              key={t}
              type="button"
              className={clsx('ink-tool', tool === t && 'is-on')}
              title={key ? `${name} (${key})` : name}
              aria-label={name}
              aria-pressed={tool === t}
              onClick={() => setTool(t)}
            >
              <Icon size={15} strokeWidth={2} />
            </button>
          ))}
          <span className="ink-dock-sep" />
          <div className="ink-color">
            <button
              type="button"
              className="ink-tool"
              title="Ink colour"
              aria-label={`Ink colour: ${COLOR_NAMES[color]}`}
              aria-expanded={paletteOpen}
              onClick={() => setPaletteOpen((o) => !o)}
            >
              <span className="ink-color-dot" style={{ background: ink(color) }} />
            </button>
            {paletteOpen && (
              <div className="ink-palette" role="group" aria-label="Ink colours">
                {INK_COLORS.map((c) => (
                  <button
                    key={c}
                    type="button"
                    className={clsx('ink-swatch', c === color && 'is-on')}
                    style={{ background: ink(c) }}
                    title={COLOR_NAMES[c]}
                    aria-label={COLOR_NAMES[c]}
                    aria-pressed={c === color}
                    onClick={() => pickColor(c)}
                  />
                ))}
              </div>
            )}
          </div>
          <button
            type="button"
            className="ink-tool"
            title="Clear all ink"
            aria-label="Clear all ink"
            disabled={strokes.length === 0}
            onClick={clearAll}
          >
            <Trash2 size={15} strokeWidth={2} />
          </button>
        </div>
      </>
    )
  })
)

const labelStyle = (x: number, y: number, size: number, color: InkColor): CSSProperties => ({
  left: x,
  top: y,
  fontSize: LABEL_FONT * size,
  lineHeight: `${LABEL_LINE * size}px`,
  height: LABEL_LINE * size,
  color: ink(color)
})

const InkLine = memo(function InkLine({ stroke, erasing }: { stroke: InkStroke; erasing: boolean }) {
  const pts = pointsOf(stroke)
  const d =
    stroke.kind === 'arrow' && pts.length > 1
      ? arrowPath(pts[0], pts[1], ARROW_HEAD * stroke.size)
      : penPath(pts)
  return (
    <path
      className={clsx('ink-line', erasing && 'is-erasing')}
      d={d}
      stroke={ink(stroke.color)}
      strokeWidth={PEN_WIDTH * stroke.size}
    />
  )
})
