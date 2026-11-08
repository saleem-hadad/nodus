// xterm.js 6's own code, without a DOM, for generate.mjs: the key evaluation,
// the Unicode 11 widths, the web-links matcher, and a headless terminal built
// on xterm's CoreTerminal (the parser, InputHandler and buffers 0.9.4 runs),
// answering colour queries as CoreBrowserTerminal does with registry.ts's theme.
import { evaluateKeyboardEvent } from 'common/input/Keyboard'
import { CoreTerminal } from 'common/CoreTerminal'
import { CellData } from 'common/buffer/CellData'
import { toRgbString } from 'common/input/XParseColor'
import { DEFAULT_ANSI_COLORS } from 'browser/Types'
import { BufferNamespaceApi } from 'common/public/BufferNamespaceApi'
import { UnicodeV11 } from '@xterm/addon-unicode11/src/UnicodeV11'
import { LinkComputer } from '@xterm/addon-web-links/src/WebLinkProvider'
import { OscLinkProvider } from 'browser/OscLinkProvider'

export { evaluateKeyboardEvent }

export const unicode11 = new UnicodeV11()

// WebLinksAddon.ts's `strictUrlRegex`, read from the source by generate.mjs.
export function computeLinks(term: Headless, y: number, regex: RegExp): Array<{ text: string; range: unknown }> {
  const api = { buffer: new BufferNamespaceApi(term as never) }
  return LinkComputer.computeLink(y, regex, api as never, () => {}).map((l) => ({ text: l.text, range: l.range }))
}

/** xterm's built-in OSC 8 link provider on line `y` (1-based). */
export function oscLinks(term: Headless, y: number): Array<{ text: string; range: unknown }> {
  const t = term as any
  const provider = new OscLinkProvider(t._bufferService, t.optionsService, t._oscLinkService)
  let links: Array<{ text: string; range: unknown }> = []
  provider.provideLinks(y, (found) => (links = (found ?? []).map((l) => ({ text: l.text, range: l.range }))))
  return links
}

type Rgb = [number, number, number]

export class Headless extends CoreTerminal {
  public replies: string[] = []
  private _ansi: Rgb[]
  private _special: { foreground: Rgb; background: Rgb; cursor: Rgb }

  constructor(
    cols: number,
    rows: number,
    private readonly _theme: { ansi: Rgb[]; foreground: Rgb; background: Rgb; cursor: Rgb }
  ) {
    // registry.ts's options that the core reads.
    super({ cols, rows, scrollback: 10000, allowProposedApi: true, cursorBlink: true, macOptionIsMeta: false })
    // CoreBrowserTerminal resets the whole terminal on RIS (`ESC c`).
    this._inputHandler.onRequestReset(() => this.reset())
    this.unicodeService.register(new UnicodeV11())
    this.unicodeService.activeVersion = '11'
    this._ansi = this._defaults()
    this._special = { foreground: _theme.foreground, background: _theme.background, cursor: _theme.cursor }
    this.onData((d) => this.replies.push(d))
    // CoreBrowserTerminal._handleColorEvent with the theme service.
    this._inputHandler.onColor((event) => {
      for (const req of event) {
        let ident: string
        let get: () => Rgb
        let set: (c: Rgb) => void
        let restore: () => void
        if (req.index === 256) {
          ident = '10'
          get = () => this._special.foreground
          set = (c) => (this._special.foreground = c)
          restore = () => (this._special.foreground = this._theme.foreground)
        } else if (req.index === 257) {
          ident = '11'
          get = () => this._special.background
          set = (c) => (this._special.background = c)
          restore = () => (this._special.background = this._theme.background)
        } else if (req.index === 258) {
          ident = '12'
          get = () => this._special.cursor
          set = (c) => (this._special.cursor = c)
          restore = () => (this._special.cursor = this._theme.cursor)
        } else {
          const i = req.index
          ident = '4;' + i
          get = () => this._ansi[i]
          set = (c) => (this._ansi[i] = c)
          restore = () => (this._ansi[i] = this._defaults()[i])
        }
        if (req.type === 0) {
          this.coreService.triggerDataEvent(`\x1b]${ident};${toRgbString(get())}\x1b\\`)
        } else if (req.type === 1) {
          set(req.color as Rgb)
        } else if (req.type === 2) {
          if (req.index === undefined) {
            // ThemeService.restoreColor(): only the 256 colours.
            this._ansi = this._defaults()
          } else {
            restore()
          }
        }
      }
    })
  }

  private _defaults(): Rgb[] {
    return DEFAULT_ANSI_COLORS.map((c, i) => {
      if (i < 16) return this._theme.ansi[i]
      const v = c.rgba
      return [(v >>> 24) & 0xff, (v >>> 16) & 0xff, (v >>> 8) & 0xff] as Rgb
    })
  }

  /** Parses `data` at once, as a finished `write` would have. */
  public feed(data: string): string[] {
    this.replies = []
    this.writeSync(data)
    return this.replies
  }

  /** Every line of the active buffer: whether it continues the one above, and each cell's characters and width. */
  public dump(): {
    cols: number
    rows: number
    cursor: [number, number]
    ybase: number
    lines: Array<{ wrapped: boolean; cells: Array<[string, number]> }>
  } {
    const buffer = this.buffers.active
    const cell = new CellData()
    const lines = []
    for (let y = 0; y < buffer.lines.length; y++) {
      const line = buffer.lines.get(y)!
      const cells: Array<[string, number]> = []
      for (let x = 0; x < this.cols; x++) {
        line.loadCell(x, cell)
        cells.push([cell.getChars(), cell.getWidth()])
      }
      lines.push({ wrapped: line.isWrapped, cells })
    }
    return { cols: this.cols, rows: this.rows, cursor: [buffer.x, buffer.y], ybase: buffer.ybase, lines }
  }

  public lineText(y: number, trim: boolean, start?: number, end?: number): string {
    return this.buffers.active.translateBufferLineToString(y, trim, start, end)
  }
}
