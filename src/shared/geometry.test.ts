import { describe, expect, it } from 'vitest'
import {
  CARD_H,
  CARD_SIZE,
  CARD_W,
  EXPANDED_H,
  EXPANDED_W,
  GAP,
  cardSize,
  findFreeSpot,
  findSpotBeside,
  intersects,
  type Point
} from './geometry'

const rect = (x: number, y: number, w = CARD_W, h = CARD_H) => ({ x, y, w, h })

describe('intersects', () => {
  it('tells overlapping rects from ones that only touch', () => {
    expect(intersects(rect(0, 0), rect(100, 100))).toBe(true)
    expect(intersects(rect(0, 0), rect(CARD_W, 0))).toBe(false)
    expect(intersects(rect(0, 0), rect(0, CARD_H + 1))).toBe(false)
  })

  it('counts rects within the margin as overlapping', () => {
    expect(intersects(rect(0, 0), rect(CARD_W + 10, 0), 11)).toBe(true)
    expect(intersects(rect(0, 0), rect(CARD_W + 10, 0), 10)).toBe(false)
  })
})

describe('cardSize', () => {
  it('is compact unless expanded, and never expanded in the archive', () => {
    expect(cardSize(undefined)).toEqual(CARD_SIZE)
    expect(cardSize({ expanded: true, archived: false })).toEqual({ w: EXPANDED_W, h: EXPANDED_H })
    expect(cardSize({ expanded: true, archived: false, w: 900, h: 600 })).toEqual({ w: 900, h: 600 })
    expect(cardSize({ expanded: true, archived: true, w: 900, h: 600 })).toEqual(CARD_SIZE)
  })
})

describe('findFreeSpot', () => {
  const clear = (p: Point, obstacles: ReturnType<typeof rect>[]) =>
    obstacles.every((o) => !intersects({ ...p, ...CARD_SIZE }, o, GAP / 2))

  it('keeps the preferred spot when it is free', () => {
    expect(findFreeSpot({ x: 0, y: 0 }, [rect(1000, 1000)])).toEqual({ x: 0, y: 0 })
  })

  it('moves to the nearest free grid cell, below before above', () => {
    const spot = findFreeSpot({ x: 0, y: 0 }, [rect(0, 0)])
    expect(spot).toEqual({ x: 0, y: CARD_H + GAP })
  })

  it('finds room among many cards', () => {
    const obstacles = [] as ReturnType<typeof rect>[]
    for (let i = -2; i <= 2; i++) for (let j = -2; j <= 2; j++) obstacles.push(rect(i * (CARD_W + GAP), j * (CARD_H + GAP)))
    const spot = findFreeSpot({ x: 0, y: 0 }, obstacles)
    expect(clear(spot, obstacles)).toBe(true)
    // The nearest ring that has room.
    expect(Math.max(Math.abs(spot.x / (CARD_W + GAP)), Math.abs(spot.y / (CARD_H + GAP)))).toBe(3)
  })

  it('keeps out of the area to avoid', () => {
    const avoid = rect(0, 0, 2000, 2000)
    const spot = findFreeSpot({ x: 100, y: 100 }, [], avoid)
    expect(intersects({ ...spot, ...CARD_SIZE }, avoid, GAP)).toBe(false)
  })

  it('fits something of another size', () => {
    const size = { w: 600, h: 400 }
    const spot = findFreeSpot({ x: 0, y: 0 }, [rect(300, 0)], undefined, size)
    expect(intersects({ ...spot, ...size }, rect(300, 0), GAP / 2)).toBe(false)
  })
})

describe('findSpotBeside', () => {
  const view = rect(0, 0, 480, 360)
  const size = { w: EXPANDED_W, h: EXPANDED_H }
  const clear = (p: Point, obstacles: ReturnType<typeof rect>[]) =>
    obstacles.every((o) => !intersects({ ...p, ...size }, o, GAP / 2))

  it('goes right of the anchor, lined up with its top', () => {
    expect(findSpotBeside(view, size, [view])).toEqual({ x: 480 + GAP, y: 0 })
  })

  it('goes below, then left, when the right is taken', () => {
    const right = rect(480 + GAP, 0)
    expect(findSpotBeside(view, size, [view, right])).toEqual({ x: 0, y: 360 + GAP })
    const below = rect(0, 360 + GAP)
    expect(findSpotBeside(view, size, [view, right, below])).toEqual({ x: -GAP - EXPANDED_W, y: 0 })
  })

  it('stays next to the anchor where findFreeSpot steps past what is in the way', () => {
    // A column of cards down the view's right side, past its bottom.
    const obstacles = [view, ...[-1, 0, 1, 2].map((i) => rect(480 + GAP, i * (CARD_H + GAP)))]
    const gap = (p: Point) =>
      Math.max(view.x - (p.x + size.w), p.x - (view.x + view.w), view.y - (p.y + size.h), p.y - (view.y + view.h))
    const spot = findSpotBeside(view, size, obstacles)
    expect(clear(spot, obstacles)).toBe(true)
    expect(gap(spot)).toBe(GAP)
    expect(gap(findFreeSpot({ x: 480 + GAP, y: 0 }, obstacles, undefined, size))).toBeGreaterThan(CARD_W)
  })

  it('keeps the gap it is given', () => {
    expect(findSpotBeside(view, size, [view], undefined, 3 * GAP)).toEqual({ x: 480 + 3 * GAP, y: 0 })
  })

  it('keeps out of the area to avoid', () => {
    const avoid = rect(480, -2000, 4000, 4000)
    const spot = findSpotBeside(view, size, [view], avoid)
    expect(intersects({ ...spot, ...size }, avoid, GAP)).toBe(false)
    expect(spot.x).toBeLessThan(0)
  })

  it('falls back to findFreeSpot when there is no room nearby', () => {
    const wall = rect(-5000, -5000, 10000, 10000)
    const spot = findSpotBeside(view, size, [wall])
    expect(clear(spot, [wall])).toBe(true)
  })
})
