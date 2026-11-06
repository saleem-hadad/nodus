// Hears Shift tapped twice in a row, in any app, from the key hook's events.
//
// A tap is Shift pressed and let go on its own: typing a capital, Shift-click
// or a shortcut with Shift in it isn't one. Two taps close together are a
// double tap, whichever Shift key made them.

/** Longest a tap can be held. */
export const TAP_MS = 350
/** Longest from letting go of the first tap to pressing the second. */
export const GAP_MS = 400

/** libuiohook's key codes for the two Shift keys, the same on every platform. */
export const SHIFT_KEYS: readonly number[] = [42, 54]

export class DoubleTap {
  /** When Shift went down, while it's down and nothing else has been pressed. */
  private downAt: number | null = null
  /** Shift is down, but something else was pressed while it was. */
  private spoiled = false
  /** When the last clean tap ended, if the next one may complete a double tap. */
  private lastTapAt: number | null = null

  constructor(private readonly onDoubleTap: () => void) {}

  keyDown(keycode: number, at: number) {
    if (!SHIFT_KEYS.includes(keycode)) return this.other()
    // Held down, the key repeats on some systems.
    if (this.downAt !== null || this.spoiled) return
    this.downAt = at
  }

  keyUp(keycode: number, at: number) {
    if (!SHIFT_KEYS.includes(keycode)) return
    const downAt = this.downAt
    const clean = downAt !== null && !this.spoiled && at - downAt <= TAP_MS
    this.downAt = null
    this.spoiled = false
    if (!clean) {
      this.lastTapAt = null
      return
    }
    if (this.lastTapAt !== null && downAt - this.lastTapAt <= GAP_MS) {
      this.lastTapAt = null
      this.onDoubleTap()
      return
    }
    this.lastTapAt = at
  }

  /** Any other key or a mouse button: a Shift held now isn't a tap, and no tap before it counts. */
  other() {
    if (this.downAt !== null) this.spoiled = true
    this.downAt = null
    this.lastTapAt = null
  }
}
