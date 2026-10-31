import { useEffect, useState } from 'react'
import { RotateCw } from 'lucide-react'
import type { UpdateStatus } from '@shared/types'

/** How updating the app is going, kept current by the main process; null until it answers. */
export function useUpdateStatus(): UpdateStatus | null {
  const [status, setStatus] = useState<UpdateStatus | null>(null)

  useEffect(() => {
    let cancelled = false
    window.api.updates.status().then((s) => !cancelled && setStatus(s))
    const unsubscribe = window.api.on.updateStatus(setStatus)
    return () => {
      cancelled = true
      unsubscribe()
    }
  }, [])

  return status
}

/** Shows in the title bar once a new version has downloaded. Otherwise it installs on quit. */
export function UpdateButton() {
  const status = useUpdateStatus()
  if (status?.state !== 'ready') return null

  return (
    <button
      className="btn btn-primary"
      onClick={() => window.api.updates.install()}
      title={`Nodus ${status.version} is ready. Sessions keep running while Nodus restarts.`}
    >
      <RotateCw size={14} strokeWidth={2.2} />
      Restart to Update
    </button>
  )
}
