import { useState } from 'react'
import { LayoutDashboard, LoaderCircle } from 'lucide-react'
import clsx from 'clsx'
import { Modal } from '@renderer/components/Modal'
import '@renderer/components/actions/actions.css'

/** Descriptions to start from. Clicking one fills the box, ready to adjust. */
const EXAMPLES = [
  {
    label: 'Vercel deployments',
    text: 'A table of the last 3 Vercel deployments of this project: the commit id, who deployed it, its status and when. Refresh every 10 minutes.'
  },
  {
    label: 'Users in the database',
    text: 'The total number of users in the database, as one big number with when it was counted. Refresh every hour.'
  },
  {
    label: 'My Linear tickets',
    text: 'All my open Linear tickets as a table: id, title, status and priority, each linking to the ticket. Refresh every hour.'
  },
  {
    label: 'Open pull requests',
    text: 'The open pull requests on GitHub for this repository, with their author, CI status and age. Refresh every 15 minutes.'
  },
  {
    label: 'Board at a glance',
    text: 'How this board is doing: how many sessions are working, waiting for me or done, and a list of the ones waiting for me that opens them when clicked.'
  }
]

/**
 * Asks what a new view should show. Claude builds it: its brain starts on the
 * description and the view shows up on the board straight away, building.
 */
export function ViewDialog({
  onCreate,
  onClose
}: {
  onCreate: (description: string) => Promise<void>
  onClose: () => void
}) {
  const [description, setDescription] = useState('')
  const [creating, setCreating] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const create = async () => {
    const what = description.trim()
    if (!what || creating) return
    setCreating(true)
    setError(null)
    try {
      await onCreate(what)
      onClose()
    } catch (err) {
      setError(
        `Couldn’t start building it: ${(err instanceof Error ? err.message : String(err)).replace(/^Error invoking remote method '[^']+': (Error: )?/, '')}`
      )
      setCreating(false)
    }
  }

  return (
    <Modal open onClose={onClose} className="action-dialog" labelledBy="view-dialog-title">
      <div
        className="action-inner"
        onKeyDown={(e) => {
          if (e.key === 'Enter' && e.metaKey) {
            e.preventDefault()
            void create()
          }
        }}
      >
        <header className="action-head">
          <div className="action-head-icon">
            <LayoutDashboard size={16} strokeWidth={2.2} />
          </div>
          <div>
            <h2 id="view-dialog-title" className="action-title">
              New view
            </h2>
            <p className="action-subtitle">
              Say what it should show and where the data comes from. Claude builds it on the board and keeps it up to
              date.
            </p>
          </div>
        </header>

        <div className="action-body">
          <textarea
            className="action-input action-description"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="For example: a table of the last 3 Vercel deployments, with the commit, who deployed it and the status."
            rows={5}
            autoFocus
            disabled={creating}
          />
          <div className="action-examples">
            <span className="action-label">Start from an example</span>
            <div className="action-chips">
              {EXAMPLES.map((ex) => (
                <button
                  key={ex.label}
                  className={clsx('action-chip', description === ex.text && 'is-active')}
                  onClick={() => setDescription(ex.text)}
                  disabled={creating}
                  title={ex.text}
                >
                  {ex.label}
                </button>
              ))}
            </div>
          </div>
          <p className="action-scope-hint" style={{ margin: 0 }}>
            Its page runs in a sandbox with no network. To fetch data it runs commands you allow on the card, or asks its
            own Claude session, which costs tokens each time.
          </p>
        </div>

        {error && <p className="action-error selectable">{error}</p>}

        <footer className="action-foot">
          <span className="action-foot-spacer" />
          <button className="btn btn-ghost" onClick={onClose} disabled={creating}>
            Cancel
          </button>
          <button className="btn btn-primary" onClick={() => void create()} disabled={!description.trim() || creating}>
            {creating && <LoaderCircle size={14} className="spin" />}
            Create view
          </button>
        </footer>
      </div>
    </Modal>
  )
}
