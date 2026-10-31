import { Loader2, Mic, Square, X } from 'lucide-react'
import clsx from 'clsx'
import { useVoice } from './useVoice'
import { VoiceNotice, VoiceStatus } from './VoiceParts'
import './voice.css'

interface Props {
  projectId: string
  /** Gets what was said, to type into the session. */
  onText: (text: string) => void
  /** Whose title bar it sits in: the session window's or an expanded card's. */
  variant: 'window' | 'card'
}

/**
 * The mic in an open session's title bar. What's said is typed into the
 * session's prompt, not sent, to finish there.
 */
export function SessionMic({ projectId, onText, variant }: Props) {
  const voice = useVoice(projectId, onText)
  const { phase } = voice
  const listening = phase === 'recording'

  return (
    <div className={clsx('session-mic', listening && 'is-recording')} ref={voice.rootRef}>
      {listening && (
        <>
          <VoiceStatus voice={voice} />
          <button className="voice-icon-btn" onClick={voice.cancel} title="Cancel (Esc)" aria-label="Cancel">
            <X size={13} />
          </button>
        </>
      )}
      <button
        className={clsx(variant === 'window' ? 'btn btn-ghost btn-icon' : 'card-tool', 'session-mic-button')}
        onClick={() => void (listening ? voice.stop() : voice.record())}
        disabled={phase === 'opening' || phase === 'transcribing'}
        title={
          listening
            ? 'Stop and type it in'
            : phase === 'transcribing'
              ? 'Transcribing…'
              : 'Speak into this session. It’s typed in for you to send.'
        }
        aria-label={listening ? 'Stop recording' : 'Speak into this session'}
      >
        {listening ? (
          <Square size={9} fill="currentColor" strokeWidth={0} />
        ) : phase === 'transcribing' ? (
          <Loader2 size={14} className="spin" />
        ) : (
          <Mic size={14} strokeWidth={2} />
        )}
      </button>
      {voice.notice && (
        <div className="session-mic-notice">
          <VoiceNotice voice={voice} />
        </div>
      )}
    </div>
  )
}
