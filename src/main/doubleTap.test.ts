import { describe, expect, it, vi } from 'vitest'
import { DoubleTap, GAP_MS, TAP_MS } from './doubleTap'

const SHIFT = 42
const RIGHT_SHIFT = 54
const A = 30

function setup() {
  const fired = vi.fn()
  return { tap: new DoubleTap(fired), fired }
}

/** Presses and lets go of `key` from `at`, held for `hold` ms. */
function press(tap: DoubleTap, key: number, at: number, hold = 60) {
  tap.keyDown(key, at)
  tap.keyUp(key, at + hold)
  return at + hold
}

describe('DoubleTap', () => {
  it('fires on two quick taps of Shift', () => {
    const { tap, fired } = setup()
    const end = press(tap, SHIFT, 0)
    press(tap, SHIFT, end + 120)
    expect(fired).toHaveBeenCalledTimes(1)
  })

  it('takes either Shift key', () => {
    const { tap, fired } = setup()
    const end = press(tap, SHIFT, 0)
    press(tap, RIGHT_SHIFT, end + 120)
    expect(fired).toHaveBeenCalledTimes(1)
  })

  it("doesn't fire on one tap", () => {
    const { tap, fired } = setup()
    press(tap, SHIFT, 0)
    expect(fired).not.toHaveBeenCalled()
  })

  it("doesn't fire when the taps are too far apart", () => {
    const { tap, fired } = setup()
    const end = press(tap, SHIFT, 0)
    press(tap, SHIFT, end + GAP_MS + 1)
    expect(fired).not.toHaveBeenCalled()
  })

  it("doesn't count Shift held too long", () => {
    const { tap, fired } = setup()
    const end = press(tap, SHIFT, 0, TAP_MS + 1)
    press(tap, SHIFT, end + 100)
    expect(fired).not.toHaveBeenCalled()
  })

  it("doesn't count typing capitals", () => {
    const { tap, fired } = setup()
    // Shift+A, then Shift+A again quickly.
    tap.keyDown(SHIFT, 0)
    press(tap, A, 10, 30)
    tap.keyUp(SHIFT, 60)
    tap.keyDown(SHIFT, 150)
    press(tap, A, 160, 30)
    tap.keyUp(SHIFT, 210)
    expect(fired).not.toHaveBeenCalled()
  })

  it('a key between the taps breaks it', () => {
    const { tap, fired } = setup()
    const end = press(tap, SHIFT, 0)
    press(tap, A, end + 40, 20)
    press(tap, SHIFT, end + 120)
    expect(fired).not.toHaveBeenCalled()
  })

  it('a mouse click between the taps breaks it', () => {
    const { tap, fired } = setup()
    const end = press(tap, SHIFT, 0)
    tap.other()
    press(tap, SHIFT, end + 120)
    expect(fired).not.toHaveBeenCalled()
  })

  it('ignores Shift repeating while held', () => {
    const { tap, fired } = setup()
    tap.keyDown(SHIFT, 0)
    tap.keyDown(SHIFT, 30)
    tap.keyDown(SHIFT, 60)
    tap.keyUp(SHIFT, 90)
    press(tap, SHIFT, 200)
    expect(fired).toHaveBeenCalledTimes(1)
  })

  it('a third tap starts over rather than firing again', () => {
    const { tap, fired } = setup()
    let end = press(tap, SHIFT, 0)
    end = press(tap, SHIFT, end + 100)
    press(tap, SHIFT, end + 100)
    expect(fired).toHaveBeenCalledTimes(1)
  })
})
