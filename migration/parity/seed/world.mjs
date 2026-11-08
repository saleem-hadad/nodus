// The synthetic world the parity scenes run on: projects, groups, boards,
// transcripts, subagents, views, actions and captures. Everything here is made
// up. Ids and times are fixed, so every run draws the same pixels.
import crypto from 'node:crypto'

/** The UI's "now": the renderer's clock is frozen here (lib/app.mjs). 2026-10-20 15:00 UTC. */
export const NOW = Date.UTC(2026, 9, 20, 15, 0, 0)
export const MIN = 60_000
export const HOUR = 60 * MIN
export const DAY = 24 * HOUR
export const ago = (ms) => NOW - ms

/** A fixed UUID for a name, so ids are the same on every run. */
export function uuid(name) {
  const h = crypto.createHash('sha1').update(`nodus-parity:${name}`).digest('hex')
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-4${h.slice(13, 16)}-a${h.slice(17, 20)}-${h.slice(20, 32)}`
}

/** A fixed hex id (agents, messages, tool uses). */
export const hex = (name, n = 17) => crypto.createHash('sha1').update(`nodus-parity:${name}`).digest('hex').slice(0, n)

export const AUTHORS = {
  avery: { name: 'Avery Quinn', email: 'avery@example.com' },
  sam: { name: 'Sam Rivera', email: 'sam@example.com' }
}

// ---------------------------------------------------------------------------
// Projects and groups

export const GROUPS = [
  { key: 'infra', name: 'Infra repos', createdAt: ago(38 * DAY) },
  { key: 'experiments', name: 'Experiments', createdAt: ago(20 * DAY), collapsed: true }
]

export const PROJECTS = [
  { key: 'storefront', color: 'blue', focused: true, openedAt: ago(25 * MIN), createdAt: ago(40 * DAY) },
  { key: 'docs-site', color: 'purple', focused: true, openedAt: ago(2 * HOUR + 10 * MIN), createdAt: ago(35 * DAY) },
  { key: 'mobile-app', color: 'pink', openedAt: ago(26 * HOUR), createdAt: ago(30 * DAY) },
  { key: 'design-system', color: 'yellow', openedAt: ago(3 * DAY), createdAt: ago(25 * DAY) },
  { key: 'infra', color: 'green', group: 'infra', focused: true, openedAt: ago(5 * HOUR), createdAt: ago(38 * DAY) },
  { key: 'terraform-modules', color: 'graphite', group: 'infra', openedAt: ago(4 * DAY), createdAt: ago(33 * DAY) },
  { key: 'k8s-config', color: 'orange', group: 'infra', openedAt: ago(6 * DAY), createdAt: ago(32 * DAY) },
  { key: 'ml-playground', color: 'red', group: 'experiments', openedAt: ago(9 * DAY), createdAt: ago(20 * DAY) },
  { key: 'ar-prototype', color: 'purple', group: 'experiments', openedAt: ago(12 * DAY), createdAt: ago(19 * DAY) },
  { key: 'legacy-api', color: 'red', archived: true, openedAt: ago(60 * DAY), createdAt: ago(200 * DAY) },
  { key: 'old-landing', color: 'graphite', archived: true, openedAt: ago(90 * DAY), createdAt: ago(240 * DAY) }
]

export const projectId = (key) => uuid(`project:${key}`)
export const groupId = (key) => uuid(`group:${key}`)
export const sessionId = (key) => uuid(`session:${key}`)
export const viewId = (key) => uuid(`view:${key}`)
export const noteId = (key) => uuid(`note:${key}`)
export const sectionId = (key) => uuid(`section:${key}`)

// ---------------------------------------------------------------------------
// Transcripts
//
// Each session is a list of steps: { user }, { text }, { tool, input }, and an
// agent step { agent: key } that calls the Agent tool. `at` is how long before
// NOW its last line was written; steps are `step` apart.

const S = (key, project, branch, at, title, steps, extra = {}) => ({ key, project, branch, at, title, steps, ...extra })

export const SESSIONS = [
  // ---- storefront: active cards ----
  S('sf-skeleton', 'storefront', 'feat/product-grid', 2 * MIN, 'Add skeleton loading to the product grid', [
    { user: 'The product grid jumps when products load. Add a skeleton state that keeps the layout steady, and use it on the search results too.' },
    { tool: 'Grep', input: { pattern: 'useProducts\\(', path: 'src' } },
    { tool: 'Read', input: { file: 'src/grid/ProductGrid.tsx' } },
    { text: "The grid renders nothing until `useProducts` resolves, so the footer jumps up and back. I'll look at how the other lists handle loading before adding a shared skeleton." },
    { agent: 'sf-a1' },
    { agent: 'sf-a2' },
    { agent: 'sf-a3' }
  ]),
  S('sf-tax', 'storefront', 'fix/tax-rounding', 6 * MIN, 'Fix rounding in checkout tax totals', [
    { user: 'Order totals are off by a cent for some baskets in the EU. Find where tax gets rounded and fix it so line items and the total agree.' },
    { tool: 'Grep', input: { pattern: 'Math.round', path: 'src/checkout' } },
    { tool: 'Read', input: { file: 'src/checkout/tax.ts' } },
    { text: 'Tax is rounded per line and again on the subtotal, so the two can drift by a cent. Rounding once, on the total, matches what the payment provider charges.' },
    { tool: 'Edit', input: { file: 'src/checkout/tax.ts', old_string: 'Math.round(line.tax * 100) / 100', new_string: 'line.tax' } },
    { text: 'I need to run the checkout tests to confirm the totals now agree. May I run `pnpm test src/checkout`?' }
  ]),
  S('sf-cart', 'storefront', 'refactor/cart-store', 48 * MIN, 'Migrate the cart store to Zustand', [
    { user: 'Move the cart from React context to a Zustand store. Keep the public hooks (useCart, useCartCount) so nothing else has to change.' },
    { tool: 'Read', input: { file: 'src/cart/CartContext.tsx' } },
    { tool: 'Write', input: { file: 'src/cart/store.ts', content: 'export const useCartStore = create<CartState>()(...)' } },
    { tool: 'Edit', input: { file: 'src/cart/hooks.ts', old_string: 'useContext(CartContext)', new_string: 'useCartStore()' } },
    { tool: 'Bash', input: { command: 'pnpm test src/cart', description: 'Run the cart tests' } },
    { text: '**Done.** The cart now lives in `src/cart/store.ts`. `useCart` and `useCartCount` read from the store with selectors, so the header badge no longer re-renders on every cart change. All 31 cart tests pass.' }
  ]),
  S('sf-coupons', 'storefront', 'test/coupons', 3 * HOUR + 30 * MIN, 'Write e2e tests for coupon codes', [
    { user: 'Write Playwright tests for coupon codes: a valid code, an expired one, and one that only applies to sale items.' },
    { tool: 'Glob', input: { pattern: 'e2e/**/*.spec.ts' } },
    { tool: 'Write', input: { file: 'e2e/coupons.spec.ts', content: "test('applies a valid coupon', async ({ page }) => {})" } },
    { tool: 'Bash', input: { command: 'pnpm playwright test e2e/coupons.spec.ts', description: 'Run the coupon tests' } },
    { text: 'Added `e2e/coupons.spec.ts` with three tests. The sale-only case caught a bug: the code was accepted for full-price items too. I left the test failing and noted it in the file so you can decide how strict it should be.' }
  ]),
  S('sf-search-plan', 'storefront', 'main', 26 * HOUR, 'Plan the v2 search page', [
    { user: 'Using what the cart and coupon sessions found, plan the v2 search page: filters, empty states, and what we can reuse.' },
    { text: '## Plan\n\n1. Reuse the skeleton tiles from the grid.\n2. Filters as a sheet on mobile, a sidebar on desktop.\n3. An empty state that suggests removing the last filter.\n\nThe cart store pattern works for the filter state too.' }
  ]),
  S('sf-run-tests', 'storefront', 'feat/product-grid', 1 * MIN, 'Run tests', [
    { user: 'Run the unit tests and fix anything that broke.' },
    { tool: 'Bash', input: { command: 'pnpm test --run', description: 'Run the unit tests' } }
  ]),

  // ---- storefront: archived (imported from disk into the archive) ----
  ...[
    ['sf-arch-01', 'Add Apple Pay to checkout', 2 * DAY, 'feat/apple-pay', 'Apple Pay now shows on Safari when the device supports it. The button sits above the card form and falls back to the card form on other browsers.'],
    ['sf-arch-02', 'Fix flaky cart drawer test', 2 * DAY + 5 * HOUR, 'main', 'The drawer test raced the slide-in animation. It now waits for the drawer to be visible before clicking, and passed 50 runs in a row.'],
    ['sf-arch-03', 'Lazy-load product images below the fold', 3 * DAY, 'perf/images', 'Images below the first two rows load lazily now. Largest Contentful Paint on the category page went from 2.9s to 1.8s on a throttled connection.'],
    ['sf-arch-04', 'Translate checkout copy to German', 3 * DAY + 7 * HOUR, 'i18n/de', 'Added `de.json` for checkout with 64 strings. Three strings need a native speaker to check the tone; I marked them with TODO.'],
    ['sf-arch-05', 'Bump Next.js to 16.2', 4 * DAY, 'chore/next-16-2', 'Upgraded to 16.2. One codemod ran for the new image config; the build and all tests pass.'],
    ['sf-arch-06', 'Investigate slow /api/search responses', 4 * DAY + 3 * HOUR, 'main', 'Search was doing a full table scan on `products.name`. A trigram index brings the p95 from 840ms to 60ms. The migration is ready but not applied.'],
    ['sf-arch-07', 'Add size guide modal to product page', 5 * DAY, 'feat/size-guide', 'The size guide opens from a link under the size picker and closes on Escape. Measurements come from the product metadata.'],
    ['sf-arch-08', 'Refactor price formatting helpers', 5 * DAY + 6 * HOUR, 'refactor/prices', 'Merged three price formatters into `formatPrice(amount, currency)`. Every call site now uses it.'],
    ['sf-arch-09', 'Set up Storybook for the design tokens', 6 * DAY, 'chore/storybook', 'Storybook runs with `pnpm storybook`. Colour, spacing and type tokens each have a page.'],
    ['sf-arch-10', 'Debug 500 on order confirmation webhook', 8 * DAY, 'fix/webhook', 'The webhook failed when an order had no shipping address (digital goods). It now skips the shipping step for those orders.'],
    ['sf-arch-11', 'Add wishlist button to product cards', 9 * DAY, 'feat/wishlist', 'Cards have a heart button that toggles the wishlist. Signed-out shoppers are asked to sign in first.'],
    ['sf-arch-12', 'Write migration for discount codes table', 11 * DAY, 'db/discounts', 'Added the `discount_codes` table with an index on `code`. The migration is reversible.'],
    ['sf-arch-13', 'Improve focus styles in the header nav', 13 * DAY, 'a11y/focus', 'Header links show a 2px focus ring now, and the skip link is the first thing Tab reaches.'],
    ['sf-arch-14', 'Remove unused Lodash imports', 15 * DAY, 'chore/lodash', 'Removed 14 unused imports; the client bundle is 9 KB smaller.'],
    ['sf-arch-15', 'Add analytics events to checkout steps', 18 * DAY, 'feat/analytics', 'Each checkout step sends a `checkout_step` event with the step name. Events are batched and skipped when the shopper opted out.'],
    ['sf-arch-16', 'Fix hydration warning on the cart badge', 23 * DAY, 'fix/hydration', 'The badge read the cart from localStorage during render. It now renders 0 on the server and fills in after mount.']
  ].map(([key, title, at, branch, reply]) =>
    S(key, 'storefront', branch, at, title, [
      { user: `${title}.` },
      { tool: 'Read', input: { file: 'src/app/layout.tsx' } },
      { text: reply }
    ])
  ),

  // ---- docs-site ----
  S('ds-api-ref', 'docs-site', 'docs/api-v3', 1 * HOUR + 30 * MIN, 'Update API reference for v3', [
    { user: 'Update the API reference pages for v3: the new pagination params and the removed /v2/legacy endpoints.' },
    { tool: 'Glob', input: { pattern: 'content/api/**/*.mdx' } },
    { text: 'Updated 12 pages. Pagination now documents `cursor` and `limit`, and the legacy endpoints are gone from the sidebar. I left a note on the storefront board about its search client.' }
  ]),
  S('ds-algolia', 'docs-site', 'main', 3 * DAY, 'Tune the search ranking for guides', [
    { user: 'Guides rank below API pages in search. Boost guides for how-to queries.' },
    { text: 'Guides get a ranking boost for queries starting with "how". The top result for "how to paginate" is now the pagination guide.' }
  ]),
  S('ds-brain-links', 'docs-site', 'main', 1 * MIN, 'Broken links report', [
    { user: 'Build the view: Broken links report. List every broken internal link with the page it is on.' },
    { tool: 'Bash', input: { command: 'pnpm lint:links --json', description: 'Check internal links' } }
  ]),
  S('ds-brain-build', 'docs-site', 'main', 40 * MIN, 'Docs build status', [
    { user: 'Build the view: Docs build status. Show the last builds with their duration and page count.' },
    { text: 'The view shows the last six builds from `build-times.json`, refreshed by the `builds` command every 10 minutes.' }
  ]),

  // ---- infra ----
  S('in-certs', 'infra', 'main', 35 * MIN, 'Rotate staging TLS certificates', [
    { user: 'Rotate the staging TLS certificates before they expire on Friday, and check every ingress picks up the new secret.' },
    { tool: 'Bash', input: { command: 'kubectl get certificates -n staging', description: 'List staging certificates' } },
    { tool: 'Bash', input: { command: 'kubectl annotate certificate api-tls cert-manager.io/issue-temporary-certificate=true -n staging', description: 'Renew the API certificate' } },
    { text: 'Renewed all four staging certificates. Every ingress serves the new chain; the oldest expires on 18 January.' }
  ]),
  S('in-alerts', 'infra', 'feat/alerts', 4 * HOUR, 'Tidy up noisy disk alerts', [
    { user: 'The disk usage alerts fire every night during backups. Make them wait 30 minutes before paging.' },
    { text: 'Added `for: 30m` to the three disk alerts and a separate warning at 90% that never pages.' }
  ]),
  S('in-budget', 'infra', 'main', 2 * DAY, 'Add a cost budget alarm', [
    { user: 'Add a monthly cost budget with an alarm at 80%.' },
    { text: 'Budget and alarm added in `budgets.tf`. The alarm emails the infra list.' }
  ]),

  // ---- the other projects: a session or two each, for the grid's counts ----
  S('dsys-tokens', 'design-system', 'main', 3 * DAY, 'Add dark mode tokens', [
    { user: 'Add dark mode values for every colour token.' },
    { text: 'Every colour token has a dark value now, checked for contrast against both surfaces.' }
  ]),
  S('dsys-button', 'design-system', 'main', 4 * DAY, 'Document the Button variants', [
    { user: 'Write docs for the Button variants.' },
    { text: 'Documented primary, secondary, ghost and danger, with do and do-not examples.' }
  ]),
  S('tf-vpc', 'terraform-modules', 'main', 4 * DAY, 'Split the VPC module', [
    { user: 'Split the VPC module into network and endpoints.' },
    { text: 'The VPC module is now two modules; existing states move with `moved` blocks.' }
  ]),
  S('k8s-hpa', 'k8s-config', 'main', 6 * DAY, 'Tune autoscaling for the API', [
    { user: 'Scale the API on request rate instead of CPU.' },
    { text: 'The API scales on requests per second through the custom metrics adapter.' }
  ]),
  S('ml-notebook', 'ml-playground', 'main', 9 * DAY, 'Clean up the embeddings notebook', [
    { user: 'Tidy the embeddings notebook so it runs top to bottom.' },
    { text: 'The notebook runs top to bottom in 40 seconds now.' }
  ]),
  S('legacy-shutdown', 'legacy-api', 'main', 60 * DAY, 'List clients still calling v1', [
    { user: 'Which clients still call the v1 API?' },
    { text: 'Two internal scripts and the old reporting job still call v1.' }
  ])
]

/** Subagents of sf-skeleton, for the agent deck and the agent viewer. */
export const AGENTS = [
  {
    key: 'sf-a1',
    session: 'sf-skeleton',
    agentType: 'Explore',
    description: 'Find every place the grid fetches products',
    status: 'working',
    steps: [
      { text: 'Looking for every caller of useProducts.' },
      { tool: 'Grep', input: { pattern: 'useProducts', path: 'src' } },
      { tool: 'Read', input: { file: 'src/search/Results.tsx' } }
    ],
    startedAgo: 1 * MIN + 30_000
  },
  {
    key: 'sf-a2',
    session: 'sf-skeleton',
    agentType: 'general-purpose',
    description: 'Write the skeleton component and its story',
    status: 'working',
    steps: [
      { text: 'Writing ProductSkeleton with the same grid as the cards.' },
      { tool: 'Write', input: { file: 'src/grid/ProductSkeleton.tsx', content: 'export function ProductSkeleton() {}' } }
    ],
    startedAgo: 1 * MIN + 10_000
  },
  {
    key: 'sf-a3',
    session: 'sf-skeleton',
    agentType: 'Explore',
    description: 'Check how other lists show loading states',
    status: 'done',
    steps: [
      { tool: 'Glob', input: { pattern: 'src/**/*Loading*.tsx' } },
      { text: 'The order history and the wishlist both use a pulsing grey block with the final layout; neither shares a component.', end: true }
    ],
    startedAgo: 1 * MIN + 50_000
  }
]

// ---------------------------------------------------------------------------
// Boards

export const BOARDS = {
  storefront: {
    archive: { x: 0, y: 0, w: 2 * 40 + 6 * 288 + 5 * 24, h: 320 },
    viewport: { x: 60, y: 96, zoom: 0.8 },
    nodes: [
      { key: 'sf-skeleton', x: 400, y: 40, autoMode: true },
      { key: 'sf-tax', x: 752, y: 40, section: 'checkout' },
      { key: 'sf-cart', x: 1072, y: 40, section: 'checkout' },
      { key: 'sf-coupons', x: 400, y: 380 },
      { key: 'sf-search-plan', x: 752, y: 380, parents: ['sf-cart', 'sf-coupons'] },
      { key: 'sf-terminal', kind: 'terminal', x: 1072, y: 380 },
      { key: 'sf-run-tests', x: 400, y: 900, runOf: 'run-tests', autoMode: true }
    ],
    sections: [{ key: 'checkout', name: 'Checkout', color: 'orange', x: 728, y: -12, createdAt: ago(2 * HOUR) }],
    notes: [
      { key: 'sf-sticker', text: 'Ship before the sale', color: 'yellow', x: 400, y: 40, session: 'sf-skeleton', createdAt: ago(3 * HOUR) },
      { key: 'sf-empty-states', text: 'Ask design about empty states for the grid and search', color: 'yellow', x: 1400, y: 40, createdAt: ago(5 * HOUR) },
      {
        key: 'sf-signed',
        text: 'The search client still calls /v2/legacy/search, which v3 removes. Switch it to /v3/search before the docs go live.',
        color: 'blue',
        x: 1400,
        y: 300,
        w: 240,
        h: 216,
        from: 'ds-api-ref',
        fromProject: 'docs-site',
        fromTitle: 'Update API reference for v3',
        createdAt: ago(80 * MIN)
      }
    ],
    ink: [
      { key: 'ink-arrow', kind: 'arrow', color: 'red', points: [1420, 250, 1372, 200], size: 1 },
      { key: 'ink-circle', kind: 'pen', color: 'blue', size: 1, circle: { cx: 896, cy: 488, rx: 190, ry: 150 } },
      { key: 'ink-label', kind: 'text', color: 'green', points: [420, 640], size: 1, text: 'ship Friday' },
      { key: 'ink-graphite', kind: 'pen', color: 'graphite', size: 1.5, wave: { x: 420, y: 700, w: 220 } }
    ]
  },
  'docs-site': {
    archive: { x: 0, y: 0, w: 2 * 40 + 6 * 288 + 5 * 24, h: 320 },
    viewport: { x: 40, y: 70, zoom: 0.62 },
    nodes: [
      { key: 'ds-api-ref', x: 400, y: 0 },
      { key: 'ds-brain-links', x: 0, y: 0, viewOf: 'ds-links' },
      { key: 'ds-brain-build', x: 0, y: 0, viewOf: 'ds-build' }
    ],
    views: [
      { key: 'ds-build', title: 'Docs build status', x: 720, y: 0, version: 3, brain: 'ds-brain-build' },
      { key: 'ds-links', title: 'Broken links report', x: 1232, y: 0, version: 0, brain: 'ds-brain-links' },
      { key: 'ds-search-size', title: 'Search index size', x: 1744, y: 0, w: 360, h: 300, version: 1 }
    ],
    notes: ['blue', 'purple', 'pink', 'red', 'orange', 'yellow', 'green', 'graphite'].map((color, i) => ({
      key: `ds-note-${color}`,
      text: [
        'Screenshots for the v3 guides',
        'Ask Sam about the changelog format',
        'Fix the dark mode code blocks',
        'Broken anchor on /api/orders',
        'Move the FAQ under Support',
        'Glossary: define “cursor”',
        'Check search after the deploy',
        'Archive the 2024 blog posts'
      ][i],
      color,
      x: 400 + (i % 4) * 248,
      y: 420 + Math.floor(i / 4) * 248,
      createdAt: ago((20 - i) * HOUR)
    }))
  },
  infra: {
    archive: { x: 0, y: 0, w: 2 * 40 + 6 * 288 + 5 * 24, h: 320 },
    viewport: { x: 30, y: 80, zoom: 0.72 },
    nodes: [
      { key: 'in-certs', x: 400, y: 0, expanded: true },
      { key: 'in-shell', kind: 'terminal', x: 1152, y: 0, expanded: true },
      { key: 'in-alerts', x: 400, y: 520 }
    ]
  },
  'design-system': {
    archive: { x: 0, y: 0, w: 2 * 40 + 6 * 288 + 5 * 24, h: 320 },
    viewport: { x: 80, y: 120, zoom: 0.9 },
    nodes: [
      { key: 'dsys-tokens', x: 400, y: 0 },
      { key: 'dsys-button', x: 720, y: 0 }
    ]
  }
}

/** Views' pages and data (userData/views/<id>/). */
export const VIEW_FILES = {
  'ds-build': {
    description: 'Show the last docs builds with their duration and page count.',
    commands: [{ name: 'builds', run: 'cat build-times.json', every: 10, approved: 'cat build-times.json' }],
    data: {
      updated: '14:52',
      builds: [
        { id: '#482', branch: 'docs/api-v3', pages: 412, seconds: 74, ok: true },
        { id: '#481', branch: 'main', pages: 409, seconds: 71, ok: true },
        { id: '#480', branch: 'docs/api-v3', pages: 409, seconds: 0, ok: false },
        { id: '#479', branch: 'main', pages: 405, seconds: 69, ok: true },
        { id: '#478', branch: 'main', pages: 405, seconds: 70, ok: true },
        { id: '#477', branch: 'fix/sidebar', pages: 404, seconds: 66, ok: true }
      ]
    }
  },
  'ds-links': { description: 'List every broken internal link with the page it is on.' },
  'ds-search-size': {
    description: 'Track the size of the search index after each deploy.',
    commands: [{ name: 'size', run: 'du -k public/search-index.json', every: 30 }],
    data: { updated: '14:30', kb: 1840, limit: 2048 }
  }
}

// ---------------------------------------------------------------------------
// Actions and captures

export const ACTIONS = {
  storefront: [
    { key: 'run-tests', name: 'Run tests', icon: 'test-tube', description: 'Run the unit tests and fix what broke', prompt: 'Run the unit tests and fix anything that broke.' },
    { key: 'open-pr', name: 'Open a PR', icon: 'git-pull-request', description: 'Open a pull request for this branch', prompt: 'Open a pull request for the current branch with a short summary of the changes.' },
    { key: 'bump-deps', name: 'Bump dependencies', icon: 'package', description: 'Update dependencies and run the tests', prompt: 'Update the dependencies to their latest minor versions and run the tests.' }
  ],
  global: [
    { key: 'daily-summary', name: 'Daily summary', icon: 'file-text', description: 'Summarise what changed today', prompt: 'Summarise what changed in this repository today, in five bullet points.' }
  ]
}

export const CAPTURES = [
  { key: 'cap-1', kind: 'session', project: 'storefront', session: 'sf-tax', title: 'Fix rounding in checkout tax totals', createdAt: ago(7 * MIN) },
  { key: 'cap-2', kind: 'note', project: 'storefront', note: 'sf-empty-states', title: 'Ask design about empty states for the grid and search', createdAt: ago(5 * HOUR) },
  { key: 'cap-3', kind: 'view', project: 'docs-site', view: 'ds-build', title: 'Docs build status', createdAt: ago(26 * HOUR) },
  { key: 'cap-4', kind: 'into', project: 'storefront', session: 'sf-cart', title: 'Migrate the cart store to Zustand', createdAt: ago(2 * DAY) }
]

/** What the claude stand-in answers its -p calls with. */
export const RESPONSES = {
  guess: {
    kind: 'session',
    project: projectId('storefront'),
    session: '',
    reason: 'It asks for work in the storefront checkout.'
  },
  draft: {
    name: 'Check bundle size',
    icon: 'gauge',
    prompt: 'Build the app, compare the bundle size of each route with main, and list any route that grew by more than 5%.'
  },
  sectionName: 'Checkout fixes',
  summary: 'Moved the cart to a Zustand store and wrote e2e tests for coupon codes; the sale-only coupon test fails on purpose.',
  summaryDelayMs: 0
}

/** Usage the scenes report through the status line hook, per session. */
export const USAGE = {
  'in-certs': {
    model: { id: 'claude-opus-5-5', display_name: 'Opus 5.5' },
    context_window: { context_window_size: 200000, current_usage: { input_tokens: 2400, cache_creation_input_tokens: 9100, cache_read_input_tokens: 72500 } },
    cost: { total_cost_usd: 1.84, total_duration_ms: 1_394_000 }
  },
  'sf-cart': {
    model: { id: 'claude-opus-5-5', display_name: 'Opus 5.5' },
    context_window: { context_window_size: 200000, current_usage: { input_tokens: 1200, cache_creation_input_tokens: 4000, cache_read_input_tokens: 31000 } },
    cost: { total_cost_usd: 0.62, total_duration_ms: 512_000 }
  }
}
