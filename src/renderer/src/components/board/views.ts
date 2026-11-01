// Geometry and React Flow nodes for views, the live pages Claude builds on the board.
import type { Node } from '@xyflow/react'
import { VIEW_H, VIEW_W, viewSize } from '@shared/geometry'
import type { BoardView, Rect } from '@shared/types'
import type { Size } from './layout'
import type { ViewFlowNode } from './ViewNode'

export { VIEW_H, VIEW_MIN_H, VIEW_MIN_W, VIEW_W, viewSize } from '@shared/geometry'

export const VIEW_SIZE: Size = { w: VIEW_W, h: VIEW_H }

/** Views sit among the cards, under notes. */
const VIEW_Z = 0

export function viewObstacles(views: BoardView[]): Rect[] {
  return views.map((v) => ({ x: v.x, y: v.y, ...viewSize(v) }))
}

const sameView = (a: BoardView, b: BoardView) => a === b || JSON.stringify(a) === JSON.stringify(b)

/**
 * React Flow nodes for the views. One being dragged or resized keeps its live
 * position and size; otherwise it sits where it was saved.
 */
export function buildViewNodes(views: BoardView[], prevById: Map<string, Node>, busy: Set<string>): ViewFlowNode[] {
  return views.map((view) => {
    const p = prevById.get(view.id) as ViewFlowNode | undefined
    const held = Boolean(p && (p.dragging || p.resizing || busy.has(view.id)))
    const size = p?.resizing ? { w: p.width!, h: p.height! } : viewSize(view)
    const position = held ? p!.position : { x: view.x, y: view.y }
    const data = p && sameView(p.data.view, view) ? p.data : { view }
    if (
      p &&
      p.data === data &&
      p.position.x === position.x &&
      p.position.y === position.y &&
      p.width === size.w &&
      p.height === size.h
    ) {
      return p
    }
    return {
      ...p,
      id: view.id,
      type: 'view',
      position,
      width: size.w,
      height: size.h,
      data,
      zIndex: VIEW_Z,
      dragHandle: '.view-head',
      selectable: true,
      deletable: false
    }
  })
}
