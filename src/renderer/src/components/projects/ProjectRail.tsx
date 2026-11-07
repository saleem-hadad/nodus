import { Fragment, useEffect, useMemo, useRef, useState, type PointerEvent } from 'react'
import clsx from 'clsx'
import { Plus, Target } from 'lucide-react'
import type { ProjectStats } from '@shared/types'
import { useApp } from '@renderer/store'
import { ContextMenu, type MenuItem } from '../ContextMenu'
import { FolderIcon } from '../FolderIcon'
import { focusModeOn, isFocusing, showsInFocus } from './focus'
import { addProject, groupProjects, projectGlyph, setFocused } from './projectsUi'
import { isMac, keys, modKey, otherModKey } from '@renderer/platform'
import './ProjectRail.css'

interface Hint {
  /** The hovered project, or null for the New Project button. */
  projectId: string | null
  top: number
}

/** Resting on the edge this long reveals the rail, so sweeping out of the window doesn't flash it. */
const REVEAL_DELAY = 100
/** Grace period after the pointer leaves, to cross the gap between the edge and the rail. */
const HIDE_DELAY = 300
/** Holding ⌘ on its own this long numbers the projects, so a quick ⌘C doesn't flash the rail. */
const NUMBERS_DELAY = 300

function activity(stats?: ProjectStats): string | undefined {
  if (!stats) return undefined
  const parts = []
  if (stats.waiting > 0) parts.push(`${stats.waiting} waiting`)
  if (stats.working > 0) parts.push(`${stats.working} working`)
  return parts.length > 0 ? parts.join(' · ') : undefined
}

/**
 * Project folders in a floating rail that auto-hides like the Dock: pointing at the
 * window's left edge slides it in over the board, to jump straight to another project.
 * Pinned (in Settings), it stays out in a column of its own beside the board.
 */
export function ProjectRail({ activeId, pinned }: { activeId: string; pinned: boolean }) {
  const projects = useApp((s) => s.projects)
  const groups = useApp((s) => s.groups)
  const stats = useApp((s) => s.stats)
  // Focus shows nowhere unless it's turned on in Settings.
  const focusFeature = useApp((s) => s.settings?.focusFeature ?? false)
  const focusMode = useApp((s) => focusModeOn(s.settings))
  const [open, setOpen] = useState(false)
  // While ⌘ is held, each project shows the number that opens it.
  const [numbered, setNumbered] = useState(false)
  const [hint, setHint] = useState<Hint | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number; projectId: string } | null>(null)
  const timer = useRef<number | undefined>(undefined)
  const railRef = useRef<HTMLElement>(null)
  const shown = pinned || open || numbered
  const focusing = isFocusing(projects, focusMode)
  const showHint = (projectId: string | null) => (e: PointerEvent<HTMLElement>) => {
    const r = e.currentTarget.getBoundingClientRect()
    setHint({ projectId, top: r.top + r.height / 2 })
  }

  // Ungrouped projects first, then one block per group. Opening a board bumps its openedAt, so
  // keep the order the projects were added in: icons must not jump around while switching.
  // While focusing, only the projects in focus, any with a session waiting on you, and this one.
  const blocks = useMemo(() => {
    const active = projects
      .filter((p) => !p.archived && (!focusing || showsInFocus(p, stats[p.id], activeId)))
      .sort((a, b) => a.createdAt - b.createdAt)
    const { ungrouped, byGroup } = groupProjects(active, groups)
    return [ungrouped, ...byGroup.map((b) => b.projects)].filter((block) => block.length > 0)
  }, [projects, groups, focusing, stats, activeId])
  // ⌘1 to ⌘9 open the first nine, top to bottom.
  const order = useMemo(() => blocks.flat(), [blocks])
  const orderRef = useRef(order)
  orderRef.current = order
  const numberOf = (projectId: string) => {
    const n = order.findIndex((p) => p.id === projectId) + 1
    return n >= 1 && n <= 9 ? n : undefined
  }

  const hinted = hint?.projectId ? projects.find((p) => p.id === hint.projectId) : undefined
  const hintName = hint && (hint.projectId ? hinted?.name : 'New Project')
  const hintGroup = hinted?.groupId ? groups.find((g) => g.id === hinted.groupId)?.name : undefined
  const hintFocus = focusing && hinted && !hinted.focused ? 'Not in focus' : undefined
  const hintNumber = hint?.projectId ? numberOf(hint.projectId) : undefined
  const hintDetail =
    hint &&
    (hint.projectId
      ? [hintGroup, activity(stats[hint.projectId]), hintFocus, hintNumber && keys(`⌘${hintNumber}`)]
          .filter(Boolean)
          .join(' · ')
      : keys('⇧⌘N'))

  // Holding ⌘ on its own numbers the projects, sliding the rail out if it's hidden, and ⌘ with a
  // number opens that project. Off a Mac, a focused terminal keeps Ctrl and its numbers.
  useEffect(() => {
    const modifier = isMac ? 'Meta' : 'Control'
    let wait: number | undefined
    const cancel = () => {
      window.clearTimeout(wait)
      wait = undefined
    }
    const hideNumbers = () => {
      cancel()
      setNumbered(false)
    }
    const blocked = (e: KeyboardEvent) =>
      e.shiftKey ||
      e.altKey ||
      otherModKey(e) ||
      useApp.getState().settingsOpen ||
      (!isMac && !!(e.target as HTMLElement | null)?.closest?.('.xterm'))

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === modifier) {
        if (!e.repeat && !blocked(e)) {
          cancel()
          wait = window.setTimeout(() => setNumbered(true), NUMBERS_DELAY)
        }
        return
      }
      // By key position, so it's the same on layouts that need Shift for digits.
      const digit = /^(?:Digit|Numpad)([1-9])$/.exec(e.code)
      if (digit && modKey(e) && !blocked(e)) {
        const project = orderRef.current[Number(digit[1]) - 1]
        if (!project) return
        e.preventDefault()
        const { route, navigate } = useApp.getState()
        if (route.view !== 'board' || route.projectId !== project.id) {
          navigate({ view: 'board', projectId: project.id })
        }
        // The numbers stay while ⌘ is held, to go on to another.
        return
      }
      // ⌘ is part of another shortcut.
      hideNumbers()
    }
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.key === modifier || !modKey(e)) hideNumbers()
    }
    window.addEventListener('keydown', onKeyDown)
    window.addEventListener('keyup', onKeyUp)
    // ⌘-click and ⌘-scroll aren't for the numbers.
    window.addEventListener('pointerdown', cancel)
    window.addEventListener('wheel', cancel, { passive: true })
    window.addEventListener('blur', hideNumbers)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
      window.removeEventListener('keyup', onKeyUp)
      window.removeEventListener('pointerdown', cancel)
      window.removeEventListener('wheel', cancel)
      window.removeEventListener('blur', hideNumbers)
      cancel()
    }
  }, [])

  // The badges need fresh stats; the projects view isn't there to load them.
  useEffect(() => {
    useApp.getState().refreshProjects()
    return () => window.clearTimeout(timer.current)
  }, [])

  const keepOpen = () => {
    window.clearTimeout(timer.current)
    timer.current = undefined
  }
  const reveal = (e: PointerEvent) => {
    if (open) return keepOpen()
    // Not while dragging a card or panning the board into the edge.
    if (e.buttons !== 0 || timer.current !== undefined) return
    timer.current = window.setTimeout(() => {
      timer.current = undefined
      setOpen(true)
    }, REVEAL_DELAY)
  }
  const hide = () => {
    keepOpen()
    // The menu sits outside the rail: going to it mustn't slide the rail away.
    if (menu) return
    timer.current = window.setTimeout(() => {
      timer.current = undefined
      setOpen(false)
      setHint(null)
    }, open ? HIDE_DELAY : 0)
  }

  const menuProject = menu ? projects.find((p) => p.id === menu.projectId) : undefined
  const menuItems: MenuItem[] = menuProject
    ? [
        {
          label: menuProject.focused ? 'Remove from Focus' : 'Add to Focus',
          icon: <Target />,
          onSelect: () => setFocused(menuProject.id, !menuProject.focused)
        }
      ]
    : []
  const closeMenu = () => {
    setMenu(null)
    if (!pinned && !railRef.current?.matches(':hover')) {
      keepOpen()
      timer.current = window.setTimeout(() => {
        timer.current = undefined
        setOpen(false)
      }, HIDE_DELAY)
    }
  }

  return (
    <>
      {!pinned && (
        <div
          className="rail-edge"
          aria-hidden="true"
          onPointerEnter={reveal}
          onPointerMove={reveal}
          onPointerLeave={hide}
        />
      )}

      <nav
        ref={railRef}
        className={clsx('rail', shown && 'is-open', pinned && 'is-pinned', numbered && 'is-numbered')}
        aria-label="Projects"
        inert={!shown}
        // Out for the numbers, it stays out under the pointer once ⌘ is let go.
        onPointerEnter={
          pinned
            ? undefined
            : () => {
                keepOpen()
                setOpen(true)
              }
        }
        onPointerLeave={pinned ? undefined : hide}
        onScroll={() => setHint(null)}
      >
        {blocks.map((block) => (
          <Fragment key={block[0].id}>
            {block.map((project) => {
              const active = project.id === activeId
              const s = stats[project.id]
              const status = s?.waiting ? 'waiting' : s?.working ? 'working' : undefined
              const number = numbered ? numberOf(project.id) : undefined
              return (
                <button
                  key={project.id}
                  className={clsx(
                    'rail-item',
                    active && 'is-active',
                    focusing && !project.focused && 'is-unfocused'
                  )}
                  aria-label={project.name}
                  aria-current={active ? 'page' : undefined}
                  onClick={() => {
                    if (!active) useApp.getState().navigate({ view: 'board', projectId: project.id })
                  }}
                  onContextMenu={
                    focusFeature
                      ? (e) => {
                          e.preventDefault()
                          setHint(null)
                          setMenu({ x: e.clientX, y: e.clientY, projectId: project.id })
                        }
                      : undefined
                  }
                  onPointerEnter={showHint(project.id)}
                  onPointerLeave={() => setHint(null)}
                >
                  <FolderIcon size={34} color={project.color} glyph={projectGlyph(project.name)} />
                  {status && <span className="status-dot rail-dot" data-status={status} />}
                  {number && (
                    <kbd className="rail-key" aria-hidden="true">
                      {number}
                    </kbd>
                  )}
                </button>
              )
            })}
            {/* Divides the groups, and the last one from the New Project button. */}
            <div className="rail-sep" aria-hidden="true" />
          </Fragment>
        ))}

        <button
          className="rail-item rail-add"
          aria-label="New Project"
          onClick={() => addProject(undefined, { open: true })}
          onPointerEnter={showHint(null)}
          onPointerLeave={() => setHint(null)}
        >
          <Plus size={18} strokeWidth={2} />
        </button>
      </nav>

      {/* Outside the rail: its transform and backdrop blur would trap a fixed tooltip. */}
      {shown && hint && hintName && (
        <div className={clsx('rail-hint', pinned && 'is-pinned')} style={{ top: hint.top }} role="tooltip">
          <span className="rail-hint-name">{hintName}</span>
          {hintDetail && <span className="rail-hint-detail">{hintDetail}</span>}
        </div>
      )}

      {menu && focusFeature && menuItems.length > 0 && (
        <ContextMenu x={menu.x} y={menu.y} items={menuItems} onClose={closeMenu} />
      )}
    </>
  )
}
