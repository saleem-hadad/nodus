// The projects grid, the title bar, Settings and the project rail.
import { board, captureOn, projectId, rest, seeds, sleep, storefront, stub } from './helpers.mjs'

const area = 'Projects grid, title bar, Settings, project rail'

const folder = (key) => `.pv-item[data-project-id="${projectId(key)}"]`

/** Turns on focus mode: offered in Settings, and switched on. */
const focusOn = (state) => {
  state.settings.focusFeature = true
  state.settings.focusMode = true
}

async function openSettings(app) {
  await captureOn(app)
  await app.menuCommand('settings')
  await app.page.waitForSelector('.settings-dialog input[name=theme]:checked')
  // The voice key field is disabled until its source is read.
  await app.page.waitForFunction(() => !document.querySelector('.settings-dialog input[type=password]:disabled'))
}

export default [
  {
    id: 'projects-empty',
    area,
    title: 'First run: no projects yet',
    specs: ['UI-183', 'UI-184', 'UI-185', 'UI-186'],
    steps: ['Seed no projects, groups or boards.', 'Launch.'],
    seed: (state) => {
      state.projects = []
      state.groups = []
      state.boards = {}
      state.captures = []
    },
    async run({ page }) {
      await page.waitForSelector('.pv-empty')
      await rest(page)
    }
  },
  {
    id: 'projects-grid',
    area,
    title: 'Populated grid: ungrouped projects with live badges, Infra repos group, collapsed Experiments group, archived section, running count',
    specs: ['UI-188', 'UI-189', 'UI-191', 'UI-193', 'UI-233', 'UI-234', 'UI-236', 'UI-104'],
    steps: [
      'Start storefront’s sessions: sf-skeleton working (two agents working), sf-tax waiting, sf-cart done but running, the terminal card’s shell.',
      'Stay on the projects grid; wait for the badges.'
    ],
    async run({ app, page }) {
      await storefront(app, { open: false })
      await page.waitForSelector(`${folder('storefront')} .pv-badge`)
      await rest(page)
    }
  },
  {
    id: 'projects-focus',
    area,
    title: 'Focus mode on: projects in focus, a waiting project shown dimmed, the hidden-count note, the title bar switch',
    specs: ['UI-195', 'UI-196', 'UI-198', 'UI-200', 'UI-201', 'UI-096'],
    steps: [
      'Seed settings.focusFeature and focusMode on (storefront, docs-site and infra are in focus).',
      'Start design-system’s dsys-tokens and make it wait on a permission (so design-system shows, dimmed).',
      'Stay on the grid.'
    ],
    seed: focusOn,
    async run({ app, page }) {
      await page.waitForSelector('.pv-item')
      await app.liven('design-system', ['dsys-tokens'])
      await app.status('dsys-tokens', 'waiting')
      await page.waitForSelector(`${folder('design-system')}.is-unfocused`)
      await rest(page)
    }
  },
  {
    id: 'projects-focus-none',
    area,
    title: 'Focus mode on with no project in focus: every project shows, with the note',
    specs: ['UI-199', 'UI-096'],
    steps: ['Seed focus mode on and no project focused.', 'Launch.'],
    seed: seeds(focusOn, (state) => state.projects.forEach((p) => delete p.focused)),
    async run({ page }) {
      await page.waitForSelector('.pv-focus-note')
      await rest(page)
    }
  },
  {
    id: 'projects-selected',
    area,
    title: 'A selected folder',
    specs: ['UI-202', 'UI-206'],
    steps: ['Click storefront’s folder once.', 'Move the pointer away.'],
    async run({ page }) {
      await page.click(folder('storefront'))
      await rest(page)
    }
  },
  {
    id: 'projects-rename',
    area,
    title: 'Renaming a project in place',
    specs: ['UI-210'],
    steps: ['Click storefront’s folder.', 'Press Return: the name becomes a field with its text selected.'],
    async run({ page }) {
      await page.click(folder('storefront'))
      await rest(page)
      await page.keyboard.press('Enter')
      await page.waitForSelector('input.pv-rename')
    }
  },
  {
    id: 'projects-context-menu',
    area,
    title: 'A project’s context menu',
    specs: ['UI-216', 'UI-217', 'UI-158', 'UI-167'],
    steps: ['Right-click storefront’s folder.'],
    async run({ page }) {
      await page.click(folder('storefront'), { button: 'right' })
      await page.waitForSelector('.ctx-menu')
    }
  },
  {
    id: 'projects-context-menu-focus',
    area,
    title: 'A project’s context menu with focus offered (Add to Focus)',
    specs: ['UI-216', 'UI-201'],
    steps: ['Seed settings.focusFeature on (focus mode off).', 'Right-click mobile-app’s folder.'],
    seed: (state) => {
      state.settings.focusFeature = true
    },
    async run({ page }) {
      await page.click(folder('mobile-app'), { button: 'right' })
      await page.waitForSelector('.ctx-menu')
    }
  },
  {
    id: 'projects-group-menu',
    area,
    title: 'A group header’s context menu',
    specs: ['UI-221', 'UI-158'],
    steps: ['Right-click the Infra repos header.'],
    async run({ page }) {
      await page.click('header.pv-group-header >> nth=0', { button: 'right' })
      await page.waitForSelector('.ctx-menu')
    }
  },
  {
    id: 'projects-group-picker',
    area,
    title: 'Move to Group… dialog',
    specs: ['UI-241', 'UI-243', 'UI-153'],
    steps: ['Right-click infra’s folder.', 'Choose Move to Group…'],
    async run({ page }) {
      await page.click(folder('infra'), { button: 'right' })
      await page.click('.ctx-item:has-text("Move to Group")')
      await page.waitForSelector('.gp[role=dialog]')
      await rest(page)
    }
  },
  {
    id: 'projects-remove-confirm',
    area,
    title: 'Remove project confirmation',
    specs: ['UI-247', 'UI-008', 'UI-153'],
    steps: ['Right-click storefront’s folder.', 'Choose Remove from Nodus…'],
    async run({ page }) {
      await page.click(folder('storefront'), { button: 'right' })
      await page.click('.ctx-item:has-text("Remove from Nodus")')
      await page.waitForSelector('.pv-confirm')
      await rest(page)
    }
  },
  {
    id: 'projects-drop',
    area,
    title: 'Dragging a folder over the window',
    specs: ['UI-228'],
    steps: ['Drag files over the grid (a dragenter carrying Files).'],
    async run({ page }) {
      await page.evaluate(() => {
        const dt = new DataTransfer()
        dt.items.add(new File(['x'], 'storefront'))
        document.querySelector('.pv').dispatchEvent(new DragEvent('dragenter', { bubbles: true, cancelable: true, dataTransfer: dt }))
      })
      await page.waitForSelector('.pv-drop')
      await rest(page)
    }
  },
  {
    id: 'projects-toast',
    area,
    title: 'The grid’s toast (a folder that couldn’t be added)',
    specs: ['UI-251', 'UI-290'],
    steps: ['Make adding a project fail (the folder picker is native, so its handler is replaced).', 'Click New Project.'],
    async run({ app, page }) {
      await page.waitForSelector('.pv-item')
      await stub(app, 'projects:add', { error: 'not a folder' })
      await page.click('#titlebar-actions button.btn')
      await page.waitForSelector('.pv-toast')
      await rest(page)
    }
  },
  {
    id: 'titlebar-running',
    area,
    title: 'The title bar’s running count, pointed at',
    specs: ['UI-095', 'UI-104', 'UI-105'],
    steps: ['Start storefront’s sessions (as in projects-grid).', 'Point at "N running" in the title bar.'],
    async run({ app, page }) {
      await storefront(app, { open: false })
      await page.hover('.titlebar-running')
      await page.waitForSelector('.titlebar-running-card')
    }
  },
  {
    id: 'titlebar-update',
    area,
    title: 'Restart to Update in the title bar (an update downloaded)',
    specs: ['UI-106', 'UI-107'],
    steps: ['Report the update state as ready (0.9.5 downloaded): the status handler and an ev:update-status event.'],
    async run({ app, page }) {
      await page.waitForSelector('.pv-item')
      const status = { state: 'ready', version: '0.9.5', current: '0.9.4' }
      await stub(app, 'updates:status', { value: status })
      await app.electron.evaluate(({ BrowserWindow }, status) => {
        for (const w of BrowserWindow.getAllWindows()) w.webContents.send('ev:update-status', status)
      }, status)
      await page.waitForSelector('button.btn-primary:has-text("Restart to Update")')
      await rest(page)
    }
  },
  {
    id: 'settings',
    area,
    title: 'Settings, top: Appearance, Focus, Floating capture (Accessibility not allowed yet)',
    specs: ['UI-116', 'UI-117', 'UI-125', 'UI-126', 'UI-129', 'UI-130', 'UI-131', 'UI-133'],
    steps: ['Open Settings (the menu’s Settings command).'],
    async run({ app, page }) {
      await page.waitForSelector('.pv-item')
      await stub(app, 'updates:status', { value: { state: 'latest', current: '0.9.4' } })
      await openSettings(app)
      await rest(page)
    }
  },
  {
    id: 'settings-bottom',
    area,
    title: 'Settings, scrolled to the end: Notifications, Voice prompts (no key), Updates (up to date)',
    specs: ['UI-136', 'UI-137', 'UI-139', 'UI-143', 'UI-147', 'UI-148', 'UI-150'],
    steps: ['Report updates as up to date (0.9.4).', 'Open Settings.', 'Scroll its body to the end.'],
    async run({ app, page }) {
      await page.waitForSelector('.pv-item')
      await stub(app, 'updates:status', { value: { state: 'latest', current: '0.9.4' } })
      await openSettings(app)
      await page.evaluate(() => {
        const body = document.querySelector('.settings-body')
        body.scrollTop = body.scrollHeight
      })
      await rest(page)
    }
  },
  {
    id: 'settings-ready',
    area,
    title: 'Settings with the double tap ready, a saved xAI key, and an update downloading',
    specs: ['UI-135', 'UI-140', 'UI-143', 'UI-148', 'UI-149'],
    steps: [
      'Report the capture shortcut as ready, the voice key as saved in Settings, and an update downloading (42%).',
      'Open Settings; scroll so Floating capture through Updates show.'
    ],
    async run({ app, page }) {
      await page.waitForSelector('.pv-item')
      await stub(app, 'capture:shortcut', { value: 'ready' })
      await stub(app, 'voice:key-source', { value: 'settings' })
      await stub(app, 'updates:status', { value: { state: 'downloading', version: '0.9.5', percent: 42, current: '0.9.4' } })
      await openSettings(app)
      await page.evaluate(() => {
        const body = document.querySelector('.settings-body')
        body.scrollTop = body.scrollHeight
      })
      await rest(page)
    }
  },
  {
    id: 'rail-open',
    area,
    title: 'The project rail slid in over the board, a project pointed at',
    specs: ['UI-253', 'UI-262', 'UI-264'],
    steps: ['Open storefront’s board.', 'Move the pointer to the window’s left edge.', 'Move onto the rail and point at docs-site.'],
    async run({ app, page }) {
      await board(app, 'storefront')
      await page.mouse.move(2, 450)
      await page.waitForSelector('nav.rail.is-open')
      await page.hover('nav[aria-label=Projects] button.rail-item[aria-label="docs-site"]')
      await page.waitForSelector('.rail-hint')
    }
  },
  {
    id: 'rail-pinned',
    area,
    title: 'The project rail pinned beside the board',
    specs: ['UI-027', 'UI-270', 'UI-271'],
    steps: ['Seed settings.pinProjectRail on.', 'Open storefront’s board.'],
    seed: (state) => {
      state.settings.pinProjectRail = true
    },
    async run({ app, page }) {
      await board(app, 'storefront')
      await page.waitForSelector('nav.rail.is-pinned')
      await rest(page)
    }
  },
  {
    id: 'rail-numbers',
    area,
    title: 'Holding ⌘: the rail’s number hints',
    specs: ['UI-275', 'UI-276'],
    steps: ['Seed settings.pinProjectRail on.', 'Open storefront’s board.', 'Hold ⌘ (Meta) for half a second.'],
    seed: (state) => {
      state.settings.pinProjectRail = true
    },
    async run({ app, page }) {
      await board(app, 'storefront')
      await rest(page)
      await page.keyboard.down('Meta')
      await page.waitForSelector('nav.rail.is-numbered')
      await sleep(300)
    }
  }
]
