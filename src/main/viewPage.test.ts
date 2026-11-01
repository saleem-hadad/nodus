import { describe, expect, it } from 'vitest'
import { VIEW_CSP, buildViewPage, scriptJson } from './viewPage'

const boot = { viewId: 'v1', title: 'Deploys', data: { n: 1 } }

/** Where the injected part (it starts with the charset meta) lands in the page. */
const injectedAt = (page: string) => page.indexOf('<meta charset="utf-8">')

describe('buildViewPage', () => {
  it('puts what every view gets at the top of the head', () => {
    const page = buildViewPage('<!doctype html><html><head><title>x</title></head><body>hi</body></html>', boot)
    expect(injectedAt(page)).toBe(page.indexOf('<head>') + '<head>'.length)
    expect(page.indexOf('window.nodus')).toBeLessThan(page.indexOf('<title>'))
  })

  it('keeps a page with no doctype out of quirks mode', () => {
    expect(buildViewPage('<html><body>hi</body></html>', boot).startsWith('<!doctype html><html><head>')).toBe(true)
    expect(buildViewPage('<p>just a fragment</p>', boot)).toMatch(/^<!doctype html><html><head>.*<body><p>just a fragment<\/p><\/body><\/html>$/s)
  })

  it('never puts anything in front of the doctype', () => {
    const page = buildViewPage('<!DOCTYPE html><body>hi</body>', boot)
    expect(page.startsWith('<!DOCTYPE html><head>')).toBe(true)
  })

  it('carries the content security policy, with no network', () => {
    const page = buildViewPage('<p>x</p>', boot)
    expect(page).toContain(`content="${VIEW_CSP}"`)
    expect(VIEW_CSP).toContain("connect-src 'none'")
    expect(VIEW_CSP).toContain("default-src 'none'")
    expect(VIEW_CSP).not.toMatch(/https?:/)
  })

  it('boots the page with its data, which can’t close the script early', () => {
    const page = buildViewPage('<p>x</p>', { ...boot, data: { html: '</script><script>alert(1)</script>' } })
    const script = page.slice(page.indexOf('<script>'), page.indexOf('</script>') + '</script>'.length)
    expect(script).toContain('"viewId":"v1"')
    expect(script).toContain('\\u003c/script>')
    // The bridge script is the only one before the page's own content.
    expect(script).not.toContain('<script>alert')
  })

  it('is not fooled by a <header> into thinking it found the head', () => {
    const page = buildViewPage('<html><header>top</header></html>', boot)
    expect(injectedAt(page)).toBeLessThan(page.indexOf('<header>'))
    expect(page).toContain('<html><head><meta charset')
  })
})

describe('scriptJson', () => {
  it('escapes what would end a script or a JS string', () => {
    expect(scriptJson('</script>')).toBe('"\\u003c/script>"')
    expect(scriptJson('a\u2028b\u2029c')).toBe('"a\\u2028b\\u2029c"')
    expect(scriptJson(undefined)).toBe('null')
  })
})
