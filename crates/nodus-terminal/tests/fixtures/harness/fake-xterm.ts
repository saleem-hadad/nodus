// Stands in for @xterm/xterm when generate.mjs runs the real registry.ts: it
// records what the registry does to its terminal (writes, pastes, resets,
// focus) and keeps the one mode the registry reads, bracketed paste. Writes
// are parsed a moment later, as xterm's WriteBuffer parses them.
const log = (...entry: unknown[]) => (globalThis as any).__log(entry)

type Listener<T> = (value: T) => void

export class Terminal {
  public cols = 80
  public rows = 24
  public modes = { bracketedPasteMode: false }
  public unicode = { activeVersion: '' }
  public _core = {}
  public sessionId = ''
  public customKeyHandler: ((ev: unknown) => boolean) | undefined
  private _data: Array<Listener<string>> = []
  private _resize: Array<Listener<{ cols: number; rows: number }>> = []
  private _parsed: Array<Listener<void>> = []
  private _selection = ''

  constructor(public options: unknown) {
    ;(globalThis as any).__terminals.push(this)
  }

  public loadAddon(addon: { activate?: (t: Terminal) => void }): void {
    addon.activate?.(this)
  }
  public host: any
  public open(host: any): void {
    this.host = host
  }
  public attachCustomWheelEventHandler(): void {}
  public attachCustomKeyEventHandler(fn: (ev: unknown) => boolean): void {
    this.customKeyHandler = fn
  }
  public onData(fn: Listener<string>) {
    this._data.push(fn)
    return { dispose: () => (this._data = this._data.filter((f) => f !== fn)) }
  }
  public onResize(fn: Listener<{ cols: number; rows: number }>) {
    this._resize.push(fn)
    return { dispose: () => (this._resize = this._resize.filter((f) => f !== fn)) }
  }
  public onWriteParsed(fn: Listener<void>) {
    this._parsed.push(fn)
    return { dispose: () => (this._parsed = this._parsed.filter((f) => f !== fn)) }
  }
  /** What the terminal sends as input (a key, a paste, a query answer). */
  public emit(data: string): void {
    for (const fn of this._data) fn(data)
  }
  public resize(cols: number, rows: number): void {
    if (cols === this.cols && rows === this.rows) return
    this.cols = cols
    this.rows = rows
    for (const fn of this._resize) fn({ cols, rows })
  }
  public write(data: string, cb?: () => void): void {
    log('term-write', this.sessionId, data)
    queueMicrotask(() => {
      const re = /\x1b\[\?2004([hl])/g
      let m
      while ((m = re.exec(data))) this.modes.bracketedPasteMode = m[1] === 'h'
      cb?.()
      for (const fn of [...this._parsed]) fn()
    })
  }
  public paste(text: string): void {
    log('term-paste', this.sessionId, text)
    let data = text.replace(/\r?\n/g, '\r')
    if (this.modes.bracketedPasteMode) data = '\x1b[200~' + data + '\x1b[201~'
    this.emit(data)
  }
  public reset(): void {
    log('term-reset', this.sessionId)
    this.modes.bracketedPasteMode = false
    this._selection = ''
  }
  public clear(): void {
    log('term-clear', this.sessionId)
  }
  public focus(): void {
    log('term-focus', this.sessionId)
  }
  public blur(): void {
    log('term-blur', this.sessionId)
  }
  public disposed = false
  public dispose(): void {
    this.disposed = true
    log('term-dispose', this.sessionId)
  }
  public setSelection(text: string): void {
    this._selection = text
  }
  public hasSelection(): boolean {
    return this._selection !== ''
  }
  public getSelection(): string {
    return this._selection
  }
  public selectAll(): void {
    log('term-select-all', this.sessionId)
  }
  public clearSelection(): void {
    log('term-clear-selection', this.sessionId)
    this._selection = ''
  }
}
export type ITheme = Record<string, string>
