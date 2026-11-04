import { useEffect } from 'react'
import { ChevronLeft, ChevronRight, Menu } from 'lucide-react'
import { useApp } from '@renderer/store'
import { FolderIcon } from './FolderIcon'
import { GitControls } from './git/GitControls'
import { TITLEBAR_ACTIONS_ID } from './TitleBarActions'
import { UpdateButton } from './UpdateButton'
import { RunningCount } from './RunningCount'
import { addProject, toggleFocusMode } from './projects/projectsUi'
import { isMac, keys } from '@renderer/platform'
import './TitleBar.css'

export function TitleBar() {
  const route = useApp((s) => s.route)
  const focusFeature = useApp((s) => s.settings?.focusFeature ?? false)
  const focusMode = useApp((s) => s.settings?.focusMode ?? false)
  const project = useApp((s) =>
    s.route.view === 'board'
      ? (s.board?.project ?? s.projects.find((p) => p.id === (s.route as { projectId: string }).projectId))
      : undefined
  )

  // Cmd+Shift+N and Cmd+Shift+F work from any view, so they are handled here rather than in ProjectsView.
  useEffect(
    () =>
      window.api.on.menu((command) => {
        if (command === 'new-project') addProject()
        else if (command === 'toggle-focus') toggleFocusMode()
      }),
    []
  )

  const goToProjects = () => useApp.getState().navigate({ view: 'projects' })

  return (
    <header className="titlebar drag">
      {/* Off a Mac there's no menu bar, so the app menu opens from here, where a Mac has its traffic lights. */}
      {!isMac && (
        <button
          className="btn btn-ghost btn-icon titlebar-menu no-drag"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect()
            void window.api.showMenu(r.left, r.bottom + 4)
          }}
          title="Menu"
          aria-label="Menu"
        >
          <Menu size={16} strokeWidth={2} />
        </button>
      )}
      <nav className="titlebar-crumbs" aria-label="Location">
        {route.view === 'board' ? (
          <>
            <button
              className="btn btn-ghost btn-icon titlebar-back no-drag"
              onClick={goToProjects}
              title="Back to Projects"
              aria-label="Back to Projects"
            >
              <ChevronLeft size={18} strokeWidth={2} />
            </button>
            <button className="titlebar-crumb-link no-drag" onClick={goToProjects}>
              Projects
            </button>
            <ChevronRight className="titlebar-crumb-sep" size={13} strokeWidth={2.2} />
            {/* no-drag: a drag region swallows double-clicks to zoom the window. */}
            <span
              className="titlebar-current no-drag"
              title={project && `${project.repoPath}\nDouble-click to ${isMac ? 'reveal in Finder' : 'show in File Explorer'}`}
              onDoubleClick={() => project && window.api.projects.revealInFinder(project.id)}
            >
              <FolderIcon size={18} color={project?.color} />
              <span className="titlebar-current-name">{project?.name ?? ''}</span>
            </span>
            <GitControls projectId={route.projectId} />
          </>
        ) : (
          <span className="titlebar-current">
            <span className="titlebar-current-name">Projects</span>
          </span>
        )}
      </nav>

      <div className="titlebar-actions no-drag">
        <RunningCount />
        {/* Only while focus mode is turned on in Settings. */}
        {focusFeature && (
          <label className="titlebar-focus" title={`Show only the projects in focus (${keys('⇧⌘F')})`}>
            Focus
            <input
              type="checkbox"
              role="switch"
              className="switch"
              checked={focusMode}
              onChange={(e) => useApp.getState().updateSettings({ focusMode: e.target.checked })}
            />
          </label>
        )}
        <UpdateButton />
        {/* Each view's own buttons. */}
        <div id={TITLEBAR_ACTIONS_ID} className="titlebar-actions" />
      </div>
    </header>
  )
}
