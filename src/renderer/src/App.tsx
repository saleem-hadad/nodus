import { useEffect } from 'react'
import clsx from 'clsx'
import type { MenuCommand } from '@shared/api'
import { isMac } from './platform'
import { useApp } from './store'
import { TitleBar } from './components/TitleBar'
import { ProjectsView } from './views/ProjectsView'
import { BoardView } from './views/BoardView'
import { TerminalModal } from './terminal/TerminalModal'
import { TranscriptPreview } from './terminal/TranscriptPreview'
import { AgentViewer } from './terminal/AgentViewer'
import { DiffModal } from './components/git/DiffModal'
import { SettingsDialog } from './components/SettingsDialog'
import { ProjectRail } from './components/projects/ProjectRail'

export function App() {
  const route = useApp((s) => s.route)
  const pinRail = useApp((s) => s.settings?.pinProjectRail ?? false)

  useEffect(() => {
    useApp.getState().refreshProjects()
    useApp.getState().loadSettings()

    const offStatus = window.api.on.status((sessionId, status, live) => {
      useApp.getState().setStatus(sessionId, status, live)
    })
    const offAgentStatus = window.api.on.agentStatus((sessionId, agentId, status) => {
      useApp.getState().setAgentStatus(sessionId, agentId, status)
    })
    const offUsage = window.api.on.usage((sessionId, usage) => {
      useApp.getState().setUsage(sessionId, usage)
    })

    // Folder badges (in the grid and the project rail) and the title bar's running count show live
    // counts, so refresh stats when statuses change. And when a process exits: SessionEnd reports a
    // session as stopped while its process is still exiting, so the exit itself changes no status.
    let statsTimer: number | undefined
    const refreshStats = () => {
      window.clearTimeout(statsTimer)
      statsTimer = window.setTimeout(() => useApp.getState().refreshProjects(), 250)
    }
    const offStatsRefresh = window.api.on.status(refreshStats)
    const offExitRefresh = window.api.on.ptyExit(refreshStats)

    const offBoard = window.api.on.boardChanged((projectId) => {
      const { route, loadBoard } = useApp.getState()
      if (route.view === 'board' && route.projectId === projectId) loadBoard(projectId)
    })
    // A session added a project with its board tools.
    const offProjects = window.api.on.projectsChanged(() => useApp.getState().refreshProjects())

    // Cmd+W closes the top-most layer and never the window itself.
    const offMenu = window.api.on.menu((command) => {
      if (command === 'settings') useApp.getState().openSettings()
      if (command !== 'close') return
      const s = useApp.getState()
      if (s.settingsOpen) s.closeSettings()
      else if (s.agentView) s.closeAgents()
      else if (s.terminalSessionId) s.closeTerminal()
      else if (s.previewSessionId) s.closePreview()
      else if (s.diffOpen) s.closeDiff()
      else if (s.route.view === 'board') s.navigate({ view: 'projects' })
    })

    // Off a Mac the menu leaves Ctrl+N, Ctrl+T and Ctrl+W to a focused terminal
    // (menu.ts), so the window takes them everywhere else.
    const CTRL_COMMANDS: Record<string, MenuCommand | undefined> = { n: 'new-session', t: 'new-terminal', w: 'close' }
    const onCtrlKey = (e: KeyboardEvent) => {
      if (isMac || !e.ctrlKey || e.shiftKey || e.altKey || e.metaKey || e.repeat || e.defaultPrevented) return
      const command = CTRL_COMMANDS[e.key.toLowerCase()]
      if (!command || (e.target as HTMLElement | null)?.closest?.('.xterm')) return
      e.preventDefault()
      void window.api.menuCommand(command)
    }
    window.addEventListener('keydown', onCtrlKey)

    // A notification the user clicked opens its card, also one clicked while the window was closed.
    const offOpenCard = window.api.on.openCard((request) => useApp.getState().showCard(request))
    window.api.takeOpenRequest().then((request) => request && useApp.getState().showCard(request))

    // Files dropped outside a drop target must not navigate the window to file://.
    const blockDrop = (e: DragEvent) => e.preventDefault()
    window.addEventListener('dragover', blockDrop)
    window.addEventListener('drop', blockDrop)

    return () => {
      window.removeEventListener('keydown', onCtrlKey)
      window.removeEventListener('dragover', blockDrop)
      window.removeEventListener('drop', blockDrop)
      offStatus()
      offAgentStatus()
      offUsage()
      offStatsRefresh()
      offExitRefresh()
      offBoard()
      offProjects()
      offMenu()
      offOpenCard()
      window.clearTimeout(statsTimer)
    }
  }, [])

  return (
    <div className="app">
      <TitleBar />
      <main className="app-main">
        {route.view === 'projects' ? (
          <ProjectsView />
        ) : (
          <>
            {/* A pinned rail takes a column of its own, so it never covers the board. */}
            <div className={clsx('board-area', pinRail && 'beside-rail')}>
              <BoardView key={route.projectId} projectId={route.projectId} />
            </div>
            <ProjectRail activeId={route.projectId} pinned={pinRail} />
          </>
        )}
      </main>
      <TerminalModal />
      <TranscriptPreview />
      <AgentViewer />
      <DiffModal />
      <SettingsDialog />
    </div>
  )
}
