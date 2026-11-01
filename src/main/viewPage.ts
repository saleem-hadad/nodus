// The page a view's frame loads: the HTML Claude wrote, with what every view
// gets put in front of it. That's a content security policy with no network,
// the board's colours as CSS variables, and `window.nodus`, the page's only
// way to the board. The frame is sandboxed without same-origin, so the page
// can't reach the board's window either; nodus posts messages to it, and the
// board answers them for this view alone (viewRelay.ts in the renderer).

/** The scheme views are served on: nodus-view://<view id>/. */
export const VIEW_SCHEME = 'nodus-view'

/**
 * Inline scripts and styles only. Nothing loads from anywhere, not even
 * another view, and the page can't send anything anywhere: the board is its
 * only way out.
 */
export const VIEW_CSP = [
  "default-src 'none'",
  "script-src 'unsafe-inline'",
  "style-src 'unsafe-inline'",
  'img-src data: blob:',
  'font-src data:',
  'media-src data: blob:',
  "connect-src 'none'",
  "frame-src 'none'",
  "worker-src 'none'",
  "form-action 'none'",
  "base-uri 'none'"
].join('; ')

/** How a view works, for its brain and for any session that puts one up. */
export const VIEW_GUIDE = `How a view works:
- It's one self-contained HTML document, shown in a sandboxed frame on a card on the board. It has no network: it can't fetch URLs or load scripts, stylesheets, fonts or images from anywhere, so put everything inline (CSS in <style>, JS in <script>, charts as inline SVG or <canvas>, icons as inline SVG or Unicode). Links open in the user's browser.
- Draw from data rather than writing values into the page: the page renders nodus.data, and you change what it shows with update_view's data, which updates the page in place. Rewrite the html only to change how it looks or works.
- Fresh data comes two ways:
  1. Commands, for anything a CLI or curl can fetch. Declare them with update_view's commands. Each runs in the project folder with the user's shell environment (their PATH, their CLIs, and env vars such as API tokens), and the page gets its output to parse. The user allows each command line once on the card before it can run. The page runs one with nodus.run(name), or give it every (minutes) to run on a schedule while the app is open. Commands cost no tokens, so prefer them, and make their output easy to parse (JSON flags, --format options, jq). Try a command yourself before declaring it.
  2. You, for what needs your tools (MCP servers such as Linear or Sentry) or judgment. Set refresh to be asked to update the view every so often (each refresh costs tokens; at least every 10 minutes), and the page can ask you with nodus.ask(prompt). Fetch what's needed, then send it with update_view's data.
- The card starts 480×360 and the user resizes it, so keep the layout fluid and readable at that size. Use the board's colours, which follow light and dark mode: var(--nodus-bg), --nodus-surface, --nodus-hover, --nodus-text, --nodus-text-2, --nodus-text-3, --nodus-border, --nodus-accent, --nodus-green, --nodus-red, --nodus-orange, --nodus-yellow, --nodus-purple, and the fonts --nodus-font and --nodus-mono. The body is 13px on var(--nodus-bg) with no margin, so add your own padding. Show when the data is from, and a clear state while there's no data yet or when a command failed.
- The page's JavaScript API is window.nodus:
  nodus.on('data', fn): calls fn with nodus.data once the page has loaded, and again whenever new data is sent.
  nodus.run(name): runs one of the view's commands, and resolves to { name, code, stdout, stderr, at, error }. nodus.results[name] holds each command's last result, and nodus.on('result', fn) gets every new one, scheduled runs included.
  nodus.state and nodus.setState(obj): the page's own saved state, such as a filter or what the user ticked, kept across reloads.
  nodus.sessions(), nodus.notes(), nodus.views(), nodus.view(id), nodus.git(): resolve to the board's sessions (id, title, status, branch, last reply), its sticky notes, the other views with their data, and the repository's git status. nodus.on('board', fn) fires when sessions, notes or other views change, and nodus.on('git', fn) when the git status does.
  nodus.ask(prompt): asks the view's brain, a Claude session of its own, to do something, such as fetching fresh data. At most once every 15 seconds.
  nodus.openSession(id), nodus.startSession(prompt), nodus.openUrl(url).
- After changing the html, check the errors update_view returns, and fix them.`

/** What the page starts with, so it can draw before asking the board for anything. */
export interface ViewBoot {
  viewId: string
  title: string
  data?: unknown
  state?: unknown
  results?: Record<string, unknown>
}

/** The board's colours (tokens.css), light then dark. A view sits on a card, so its ground is the card's. */
const THEME = `
:root {
  color-scheme: light dark;
  --nodus-bg: #ffffff;
  --nodus-surface: rgba(0, 0, 0, 0.035);
  --nodus-hover: rgba(0, 0, 0, 0.05);
  --nodus-text: rgba(0, 0, 0, 0.88);
  --nodus-text-2: rgba(0, 0, 0, 0.56);
  --nodus-text-3: rgba(0, 0, 0, 0.36);
  --nodus-border: rgba(0, 0, 0, 0.1);
  --nodus-accent: #007aff;
  --nodus-green: #2da44e;
  --nodus-red: #cf222e;
  --nodus-orange: #ff9500;
  --nodus-yellow: #d4a72c;
  --nodus-purple: #8250df;
  --nodus-font: -apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Helvetica Neue', sans-serif;
  --nodus-mono: 'SF Mono', ui-monospace, Menlo, Monaco, monospace;
}
@media (prefers-color-scheme: dark) {
  :root {
    --nodus-bg: #2a2a2d;
    --nodus-surface: rgba(255, 255, 255, 0.04);
    --nodus-hover: rgba(255, 255, 255, 0.07);
    --nodus-text: rgba(255, 255, 255, 0.9);
    --nodus-text-2: rgba(255, 255, 255, 0.58);
    --nodus-text-3: rgba(255, 255, 255, 0.36);
    --nodus-border: rgba(255, 255, 255, 0.1);
    --nodus-accent: #0a84ff;
    --nodus-green: #3fb950;
    --nodus-red: #f85149;
    --nodus-orange: #ff9f0a;
    --nodus-yellow: #e3b341;
    --nodus-purple: #a371f7;
  }
}
:where(html, body) {
  margin: 0;
  background: var(--nodus-bg);
  color: var(--nodus-text);
  font: 13px/1.45 var(--nodus-font);
  -webkit-font-smoothing: antialiased;
}
:where(a) { color: var(--nodus-accent); }
`

/**
 * window.nodus. Requests go to the board as { nodus: 1, id, method, args } and
 * come back with the same id; events arrive as { nodus: 1, event, payload }.
 * `on('data' | 'state', fn)` calls fn with what there is now, then on every
 * change, so one render function covers both.
 */
const BRIDGE = `
(function () {
  var boot = __BOOT__;
  var seq = 0, pending = {}, listeners = {};
  var current = { data: boot.data, state: boot.state, results: boot.results || {} };
  function post(msg) { msg.nodus = 1; parent.postMessage(msg, '*'); }
  function call(method, args) {
    return new Promise(function (resolve, reject) {
      var id = ++seq;
      pending[id] = { resolve: resolve, reject: reject };
      post({ id: id, method: method, args: args || {} });
    });
  }
  function report(err) {
    var text = err && (err.stack || err.message) ? (err.stack || err.message) : String(err);
    post({ error: String(text).slice(0, 2000) });
  }
  function emit(event, payload) {
    (listeners[event] || []).slice().forEach(function (fn) {
      try { fn(payload); } catch (err) { report(err); }
    });
  }
  function whenReady(fn) {
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', fn, { once: true });
    else Promise.resolve().then(fn);
  }
  window.addEventListener('message', function (e) {
    if (e.source !== parent) return;
    var m = e.data;
    if (!m || m.nodus !== 1) return;
    if (m.id) {
      var p = pending[m.id];
      if (!p) return;
      delete pending[m.id];
      if (m.error) p.reject(new Error(m.error)); else p.resolve(m.result);
      return;
    }
    if (m.event === 'data') { current.data = m.payload; nodus.data = m.payload; }
    else if (m.event === 'state') { current.state = m.payload; nodus.state = m.payload; }
    else if (m.event === 'result' && m.payload) { current.results[m.payload.name] = m.payload; }
    if (m.event) emit(m.event, m.payload);
  });
  var nodus = {
    viewId: boot.viewId,
    title: boot.title,
    data: current.data,
    state: current.state,
    results: current.results,
    on: function (event, fn) {
      (listeners[event] = listeners[event] || []).push(fn);
      if (event === 'data' || event === 'state') {
        whenReady(function () {
          if (current[event] !== undefined && (listeners[event] || []).indexOf(fn) >= 0) {
            try { fn(current[event]); } catch (err) { report(err); }
          }
        });
      }
      return function () {
        listeners[event] = (listeners[event] || []).filter(function (f) { return f !== fn; });
      };
    },
    setState: function (state) {
      current.state = state;
      nodus.state = state;
      return call('setState', { state: state });
    },
    run: function (name) { return call('run', { name: name }); },
    ask: function (prompt) { return call('ask', { prompt: prompt }); },
    sessions: function () { return call('sessions'); },
    notes: function () { return call('notes'); },
    views: function () { return call('views'); },
    view: function (id) { return call('view', { id: id }); },
    git: function () { return call('git'); },
    openSession: function (id) { return call('openSession', { id: id }); },
    startSession: function (prompt) { return call('startSession', { prompt: prompt }); },
    openUrl: function (url) { return call('openUrl', { url: url }); }
  };
  window.nodus = nodus;
  window.addEventListener('error', function (e) { report(e.error || e.message); });
  window.addEventListener('unhandledrejection', function (e) { report(e.reason); });
  window.addEventListener('load', function () { post({ loaded: true }); });
  // Links open in the browser. The frame itself never leaves the view.
  document.addEventListener('click', function (e) {
    var a = e.target && e.target.closest ? e.target.closest('a[href]') : null;
    if (!a) return;
    var href = a.getAttribute('href') || '';
    if (href.charAt(0) === '#') return;
    e.preventDefault();
    if (/^https?:/i.test(a.href)) call('openUrl', { url: a.href });
  }, true);
})();
`

/** JSON that's safe inside a <script>: no `</script>`, and no line separators that end a JS string. */
export function scriptJson(value: unknown): string {
  return (JSON.stringify(value) ?? 'null')
    .replace(/</g, '\\u003c')
    .replace(/\u2028/g, '\\u2028')
    .replace(/\u2029/g, '\\u2029')
}

/**
 * The page as served: what every view gets, at the top of its <head>, then
 * the HTML Claude wrote. Put in front of the doctype it would switch the page
 * into quirks mode, so it goes after it.
 */
export function buildViewPage(html: string, boot: ViewBoot): string {
  const injected =
    `<meta charset="utf-8">` +
    `<meta http-equiv="Content-Security-Policy" content="${VIEW_CSP}">` +
    `<meta name="viewport" content="width=device-width, initial-scale=1">` +
    `<style>${THEME}</style>` +
    `<script>${BRIDGE.replace('__BOOT__', () => scriptJson(boot))}</script>`
  const insertAfter = (match: RegExpMatchArray, text: string) =>
    html.slice(0, match.index! + match[0].length) + text + html.slice(match.index! + match[0].length)

  const doctype = html.match(/<!doctype[^>]*>/i)
  // Without one the page would render in quirks mode.
  const standards = (page: string) => (doctype ? page : `<!doctype html>${page}`)
  const head = html.match(/<head(\s[^>]*)?>/i)
  if (head) return standards(insertAfter(head, injected))
  const root = html.match(/<html(\s[^>]*)?>/i)
  if (root) return standards(insertAfter(root, `<head>${injected}</head>`))
  if (doctype) return insertAfter(doctype, `<head>${injected}</head>`)
  return `<!doctype html><html><head>${injected}</head><body>${html}</body></html>`
}

/** What a view without a page yet would show, were its frame loaded anyway. */
export const EMPTY_PAGE = '<!doctype html><html><head></head><body></body></html>'
