import { describe, expect, it } from 'vitest'
import { CARD_H, CARD_SIZE, CARD_W, EXPANDED_H, EXPANDED_W, GAP, cardSize, findFreeSpot, intersects, type Point } from './geometry'

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
