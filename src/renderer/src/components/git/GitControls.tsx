import { useEffect, useRef, useState } from 'react'
import { FileDiff, GitBranch } from 'lucide-react'
import type { GitCommit, GitStatus } from '@shared/types'
import { useApp } from '@renderer/store'
import { timeAgo } from '../board/layout'
import './git.css'

/** How often the branch and change count are re-read while the window has focus. */
export const GIT_POLL_MS = 4000

/** Branch chip and Changes button, shown next to the project name in the title bar. */
export function GitControls({ projectId }: { projectId: string }) {
  const git = useApp((s) => s.git)
  const openDiff = useApp((s) => s.openDiff)

  useEffect(() => {
    const refresh = () => useApp.getState().refreshGit()
    refresh()
    // Claude edits files during a turn, so a status change usually means new changes.
    const offStatus = window.api.on.status(refresh)
    window.addEventListener('focus', refresh)
    // Catches branch switches and edits made in terminals or other apps.
    const timer = window.setInterval(() => {
      if (document.hasFocus()) refresh()
    }, GIT_POLL_MS)
    return () => {
      offStatus()
      window.removeEventListener('focus', refresh)
      window.clearInterval(timer)
    }
  }, [projectId])

  if (!git) return null
  const count = git.files.length

  return (
    <>
      <BranchChip projectId={projectId} git={git} />
      <button
        className="git-changes no-drag"
        onClick={openDiff}
        title={count ? `Show ${count} changed ${count === 1 ? 'file' : 'files'}` : 'No local changes'}
      >
        <FileDiff size={14} strokeWidth={2} />
        Changes
        {count > 0 && <span className="git-count">{count > 999 ? '999+' : count}</span>}
      </button>
    </>
  )
}

/** Pointing at the branch waits this long before the log opens, so passing over it does nothing. */
const OPEN_DELAY_MS = 150
/** Leaving waits this long before it closes, so the pointer can cross into the log. */
const CLOSE_DELAY_MS = 150

/** The branch, with its last few commits dropping down while it's pointed at. */
function BranchChip({ projectId, git }: { projectId: string; git: GitStatus }) {
  const [open, setOpen] = useState(false)
  const [log, setLog] = useState<{ key: string; commits: GitCommit[] } | null>(null)
  const timer = useRef<number | undefined>(undefined)
  // A new commit, amend, reset or checkout moves HEAD, and with it the log.
  const key = `${projectId}:${git.oid}`

  useEffect(() => {
    if (!open || !git.oid || log?.key === key) return
    let live = true
    window.api.git.log(projectId).then(
      (commits) => {
        if (live) setLog({ key, commits })
      },
      () => {}
    )
    return () => {
      live = false
    }
  }, [open, key])

  useEffect(() => () => window.clearTimeout(timer.current), [])

  const openSoon = (wanted: boolean) => {
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => setOpen(wanted), wanted ? OPEN_DELAY_MS : CLOSE_DELAY_MS)
  }

  const commits = log?.key === key ? log.commits : null

  return (
    // no-drag: the title bar is a drag region, which would swallow the pointer.
    <div
      className="git-branch-wrap no-drag"
      onPointerEnter={() => openSoon(true)}
      onPointerLeave={() => openSoon(false)}
    >
      <span className="git-branch">
        <GitBranch size={13} strokeWidth={2.2} />
        <span className="git-branch-name">{branchLabel(git)}</span>
        {git.ahead > 0 && <span className="git-branch-ab">↑{git.ahead}</span>}
        {git.behind > 0 && <span className="git-branch-ab">↓{git.behind}</span>}
      </span>
      {open && (
        <div className="git-log" role="tooltip">
          <div className="git-log-head">
            <span className="git-log-branch">{git.branch ?? 'Detached HEAD'}</span>
            <span className="git-log-tracking">{trackingLabel(git)}</span>
          </div>
          {git.oid && commits && commits.length > 0 && (
            <ol className="git-log-list">
              {commits.map((c) => (
                <li key={c.oid} className="git-log-commit">
                  <span className="git-log-subject" title={c.subject}>
                    {c.subject}
                  </span>
                  <span className="git-log-time">{timeAgo(c.time)}</span>
                  <span className="git-log-meta">
                    <span className="git-log-oid">{c.oid.slice(0, 7)}</span> · {c.author}
                  </span>
                </li>
              ))}
            </ol>
          )}
        </div>
      )}
    </div>
  )
}

export function branchLabel(git: GitStatus): string {
  return git.branch ?? git.oid?.slice(0, 7) ?? 'HEAD'
}

function trackingLabel(git: GitStatus): string {
  if (!git.oid) return 'No commits yet'
  if (!git.branch) return `At ${git.oid.slice(0, 7)}`
  if (!git.upstream) return 'Not published to a remote'
  const ab = [git.ahead && `${git.ahead} ahead`, git.behind && `${git.behind} behind`].filter(Boolean)
  return `Tracking ${git.upstream}${ab.length ? ` · ${ab.join(', ')}` : ', up to date'}`
}
