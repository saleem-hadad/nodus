// Board geometry the main process needs as well as the renderer: the sizes of
// cards, notes, views and the archive dock, and the search for a free spot.
// The renderer draws the board; the main process places the notes and views
// that sessions put up with their board tools.
import type { BoardNode, BoardNote, BoardView, Rect } from './types'

export interface Point {
  x: number
  y: number
}

export interface Size {
  w: number
  h: number
}

/** Card size in flow units; matches --card-w / --card-h (4:3). */
export const CARD_W = 288
export const CARD_H = 216
export const GAP = 32
export const CARD_SIZE: Size = { w: CARD_W, h: CARD_H }

/** An expanded card starts with room for an 80-column terminal. */
export const EXPANDED_W = 720
export const EXPANDED_H = 480

/** Height of the archive header strip. */
export const ARCHIVE_HEADER_H = 52
export const ARCHIVE_PAD = 28
/** The search field sits under the stack, below the peeking cards. */
export const STACK_SEARCH_Y = ARCHIVE_HEADER_H + CARD_H + 38
export const STACK_SEARCH_H = 34
export const STACK_W = ARCHIVE_PAD * 2 + CARD_W
export const STACK_H = STACK_SEARCH_Y + STACK_SEARCH_H + 22

/** A note starts square, as tall as a card so the two line up in rows. */
export const NOTE_W = CARD_H
export const NOTE_H = CARD_H

/** The pad sits under the archive stack, as wide as it, with a header like the archive's. */
export const PAD_HEADER_H = ARCHIVE_HEADER_H
export const PAD_W = STACK_W
const PAD_GAP = 24
/** Room under the top sheet for the sheets beneath it to show. */
const PAD_BOTTOM = ARCHIVE_PAD

/** Whether a card is drawn expanded, with its terminal on the board. Archived cards never are. */
export function isExpanded(card: Pick<BoardNode, 'expanded' | 'archived'> | undefined): boolean {
  return Boolean(card?.expanded && !card.archived)
}

/** A card's size on the board: compact, or as big as it was last expanded to. */
export function cardSize(card: Pick<BoardNode, 'expanded' | 'archived' | 'w' | 'h'> | undefined): Size {
  if (!isExpanded(card)) return CARD_SIZE
  return { w: card!.w ?? EXPANDED_W, h: card!.h ?? EXPANDED_H }
}

export function noteSize(note: Pick<BoardNote, 'w' | 'h'>): Size {
  return { w: note.w ?? NOTE_W, h: note.h ?? NOTE_H }
}

/** A view starts with room for a small table, and can be resized from 240×160 to 2400×1800. */
export const VIEW_W = 480
export const VIEW_H = 360
export const VIEW_MIN_W = 240
export const VIEW_MIN_H = 160
export const VIEW_MAX_W = 2400
export const VIEW_MAX_H = 1800

export function viewSize(view: Pick<BoardView, 'w' | 'h'>): Size {
  return { w: view.w ?? VIEW_W, h: view.h ?? VIEW_H }
}

export const clampViewSize = (size: Partial<Size>): Partial<Size> => ({
  w: Number.isFinite(size.w) ? Math.round(Math.min(VIEW_MAX_W, Math.max(VIEW_MIN_W, size.w!))) : undefined,
  h: Number.isFinite(size.h) ? Math.round(Math.min(VIEW_MAX_H, Math.max(VIEW_MIN_H, size.h!))) : undefined
})

/** The pad under the archive's stack. It goes by the folded stack, so the open panel floats over it. */
export function notePadRect(archive: Rect): Rect {
  return { x: archive.x, y: archive.y + STACK_H + PAD_GAP, w: PAD_W, h: PAD_HEADER_H + NOTE_H + PAD_BOTTOM }
}

/** The folded archive stack and the pad under it, which new cards and notes keep clear of. */
export function dockRect(archive: Rect): Rect {
  const pad = notePadRect(archive)
  return { x: archive.x, y: archive.y, w: STACK_W, h: pad.y + pad.h - archive.y }
}

export function intersects(a: Rect, b: Rect, margin = 0): boolean {
  return (
    a.x < b.x + b.w + margin &&
    a.x + a.w + margin > b.x &&
    a.y < b.y + b.h + margin &&
    a.y + a.h + margin > b.y
  )
}

/**
 * Finds a free spot for a card (or anything of `size`) near `preferred` that
 * overlaps neither the given obstacles nor (optionally) the archive zone. It
 * searches rings of card-sized grid cells around the preferred position,
 * closest first.
 */
export function findFreeSpot(preferred: Point, obstacles: Rect[], avoid?: Rect, size: Size = CARD_SIZE): Point {
  const stepX = CARD_W + GAP
  const stepY = CARD_H + GAP
  const blocked = (p: Point) => {
    const r = { ...p, ...size }
    if (avoid && intersects(r, avoid, GAP)) return true
    return obstacles.some((o) => intersects(r, o, GAP / 2))
  }
  if (!blocked(preferred)) return preferred

  for (let ring = 1; ring < 40; ring++) {
    const candidates: Point[] = []
    for (let dx = -ring; dx <= ring; dx++) {
      for (let dy = -ring; dy <= ring; dy++) {
        if (Math.max(Math.abs(dx), Math.abs(dy)) !== ring) continue
        candidates.push({ x: preferred.x + dx * stepX, y: preferred.y + dy * stepY })
      }
    }
    // Prefer spots to the right of / below the preferred point, then nearest.
    candidates.sort((a, b) => score(a) - score(b))
    const hit = candidates.find((c) => !blocked(c))
    if (hit) return hit
  }
  return { x: preferred.x, y: preferred.y + 40 * stepY }

  function score(p: Point) {
    const dx = p.x - preferred.x
    const dy = p.y - preferred.y
    const behind = (dx < 0 ? 1.4 : 1) * (dy < 0 ? 1.2 : 1)
    return Math.hypot(dx, dy) * behind
  }
}

/**
 * Finds a free spot for something of `size` right beside `anchor`, `gap`
 * away: to its right if there's room, else below, left of or above it, and
 * lined up with it where it can be; then further out. Unlike findFreeSpot,
 * which steps a whole card at a time, it slides along the anchor's sides, so
 * what's placed lands next to the anchor even when the board around it is
 * busy. Further than twice its size away, it falls back to findFreeSpot.
 */
export function findSpotBeside(anchor: Rect, size: Size, obstacles: Rect[], avoid?: Rect, gap = GAP): Point {
  const reach = 2 * Math.max(size.w, size.h)
  const right = anchor.x + anchor.w + gap
  const left = anchor.x - gap - size.w
  const below = anchor.y + anchor.h + gap
  const above = anchor.y - gap - size.h
  // Positions on one axis: out from either side of the anchor, and along it in between.
  const axis = (start: number, before: number, after: number) => {
    const out: number[] = []
    for (let d = 0; d <= reach; d += GAP) out.push(after + d, before - d)
    for (let v = start - Math.floor((start - before) / GAP) * GAP; v < after; v += GAP) out.push(v)
    return out
  }
  const xs = axis(anchor.x, left, right)
  const ys = axis(anchor.y, above, below)

  let best: Point | null = null
  let bestScore = Infinity
  for (const x of xs) {
    for (const y of ys) {
      const gx = Math.max(anchor.x - (x + size.w), x - (anchor.x + anchor.w))
      const gy = Math.max(anchor.y - (y + size.h), y - (anchor.y + anchor.h))
      if (Math.max(gx, gy) < gap - 1) continue
      // Beside it rather than above or below it, and how far it is from lining up.
      const sideways = gx >= gy
      const weight = sideways ? (x > anchor.x ? 1 : 1.3) : y > anchor.y ? 1.15 : 1.45
      const offset = sideways ? Math.abs(y - anchor.y) : Math.abs(x - anchor.x)
      const score = Math.hypot(Math.max(gx, 0), Math.max(gy, 0)) * weight + offset / 2
      if (score >= bestScore) continue
      const r = { x, y, ...size }
      if (avoid && intersects(r, avoid, GAP)) continue
      if (obstacles.some((o) => intersects(r, o, GAP / 2))) continue
      best = { x, y }
      bestScore = score
    }
  }
  return best ?? findFreeSpot({ x: right, y: anchor.y }, obstacles, avoid, size)
}
