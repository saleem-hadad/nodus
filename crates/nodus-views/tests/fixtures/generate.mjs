// Writes page.json beside this file from the TypeScript itself, so the Rust
// tests check the page a view's frame loads byte for byte against 0.9.4's:
// the CSP, the theme, the bridge script, the guide on macOS and Windows, and
// where the injected part goes in pages of every shape. Everything is made up.
//
// The committed page.json was generated from 0.9.4's sources. The Electron
// app has since left this repo, so to regenerate it, point NODUS_V094 at a
// checkout of tag v0.9.4 and run, from this repo's root, with Node 22:
//
//   git worktree add ../nodus-0.9.4 v0.9.4
//   NODUS_V094=../nodus-0.9.4 node --experimental-strip-types crates/nodus-views/tests/fixtures/generate.mjs
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
if (!process.env.NODUS_V094) throw new Error('Set NODUS_V094 to a checkout of tag v0.9.4 (see the top of this file)')
const root = path.resolve(process.env.NODUS_V094)

async function load(platform) {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), `nodus-page-${platform}-`))
  const find = `process.platform === 'win32'`
  let source = fs.readFileSync(path.join(root, 'src/main/viewPage.ts'), 'utf8')
  if (!source.includes(find)) throw new Error(`viewPage.ts no longer has: ${find}`)
  source = source.replace(find, `${JSON.stringify(platform)} === 'win32'`)
  fs.writeFileSync(path.join(work, 'viewPage.mts'), source)
  return import(pathToFileURL(path.join(work, 'viewPage.mts')).href)
}

const darwin = await load('darwin')
const win32 = await load('win32')

const pages = [
  '<!doctype html><html><head><title>x</title></head><body>hi</body></html>',
  '<html><body>hi</body></html>',
  '<p>just a fragment</p>',
  '<!DOCTYPE html><body>hi</body>',
  '<html><header>top</header></html>',
  '<HTML lang="en"><HEAD\n  data-x="1"><TITLE>T</TITLE></HEAD><BODY></BODY></HTML>',
  '<!doctype html><html><head',
  '<!doctype html',
  '<header><head>after a header</head>',
  '<body><p>a body first</p><html><head></head></html>',
  '',
  '<head\t>tab</head>',
  '<head x>no-break space</head>',
  '<head\u0085>next line, which isn’t a space to JavaScript</head><html>',
  '<html lang="x"><body>line separator</body></html>',
  '<htmlx><headx>neither</headx></htmlx>',
  '<p>$& $1 $$ __BOOT__</p>',
  'é😀<head>unicode before the head</head>',
  '<!-- <head> in a comment --><!doctype html><html><head></head></html>',
  '<!doctype html>\n<html>\n<head>\n<meta charset="utf-8">\n</head>\n<body></body>\n</html>\n'
]

const boots = [
  { viewId: 'v1', title: 'Deploys', data: { n: 1 } },
  {
    viewId: '7c9e6679-7425-40de-944b-e07fc1f90ae7',
    title: 'Tests “quoted” </script>',
    startsSessions: true,
    data: { html: '</script><script>alert(1)</script>', sep: 'a b c', at: 1_700_000_000_000, f: 0.1, emoji: '😀' },
    state: { filter: 'open', list: [] },
    results: { count: { name: 'count', code: null, stdout: '$&', stderr: '', at: 1.5, durationMs: 3 } }
  },
  { viewId: 'v2', title: '', data: null, results: {} }
]

const scriptJson = ['</script>', 'a b c', undefined, null, 1e21, -0, { a: [1, '<'] }, 'plain'].map(
  (value) => darwin.scriptJson(value)
)

const out = {
  scheme: darwin.VIEW_SCHEME,
  csp: darwin.VIEW_CSP,
  emptyPage: darwin.EMPTY_PAGE,
  guide: { darwin: darwin.VIEW_GUIDE, win32: win32.VIEW_GUIDE },
  scriptJson,
  // Every page with the first boot, and every boot with the first page.
  pages: [
    ...pages.map((html) => ({ html, boot: boots[0] })),
    ...boots.slice(1).map((boot) => ({ html: pages[0], boot }))
  ].map(({ html, boot }) => ({ html, boot, page: darwin.buildViewPage(html, boot) }))
}
fs.writeFileSync(path.join(here, 'page.json'), JSON.stringify(out, null, 2) + '\n')
console.log('wrote page.json')
