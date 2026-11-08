// Stands in for xterm's addons when generate.mjs runs registry.ts. The fit
// addon sizes the terminal from the size the scenario gave its container, or
// throws while it has none (not laid out), as fit() does.
export class FitAddon {
  private term: any
  public activate(term: any): void {
    this.term = term
  }
  public fit(): void {
    const size = this.term?.host?.parentElement?.size
    if (!size) throw new Error('not laid out')
    this.term.resize(size[0], size[1])
  }
  public dispose(): void {}
}
export class WebglAddon {
  public activate(): void {}
  public onContextLoss(): void {}
  public dispose(): void {}
}
export class Unicode11Addon {
  public activate(): void {}
}
export class WebLinksAddon {
  constructor(public handler: unknown) {}
  public activate(): void {}
}
