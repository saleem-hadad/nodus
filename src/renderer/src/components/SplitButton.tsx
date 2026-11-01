import { useCallback, useRef, useState, type ReactNode } from 'react'
import { ChevronDown } from 'lucide-react'
import clsx from 'clsx'
import { ContextMenu, type MenuItem } from './ContextMenu'
import './SplitButton.css'

interface Props {
  /** The default action's button content. */
  children: ReactNode
  onClick: () => void
  title?: string
  /** The other actions, in the menu dropped from the chevron. */
  items: MenuItem[]
  menuLabel?: string
}

/** A button for the default action, with a chevron beside it that drops a menu of the others. */
export function SplitButton({ children, onClick, title, items, menuLabel = 'More' }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  const moreRef = useRef<HTMLButtonElement>(null)
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null)
  const close = useCallback(() => setMenu(null), [])

  const toggle = () => {
    if (menu) return close()
    const r = ref.current!.getBoundingClientRect()
    setMenu({ x: r.right, y: r.bottom + 4 })
  }

  return (
    <div ref={ref} className={clsx('split-btn no-drag', menu && 'is-open')}>
      <button className="btn split-btn-main" onClick={onClick} title={title}>
        {children}
      </button>
      <button
        ref={moreRef}
        className="btn split-btn-more"
        aria-label={menuLabel}
        aria-haspopup="menu"
        aria-expanded={!!menu}
        title={menuLabel}
        onPointerDown={toggle}
        // Keyboard presses arrive as clicks with no pointer behind them.
        onClick={(e) => e.detail === 0 && toggle()}
      >
        <ChevronDown size={13} strokeWidth={2.2} />
      </button>
      {menu && <ContextMenu x={menu.x} y={menu.y} align="right" anchor={moreRef} items={items} onClose={close} />}
    </div>
  )
}
