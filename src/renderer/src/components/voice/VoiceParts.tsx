import { AlertCircle, Loader2, RotateCcw, Settings, X } from 'lucide-react'
import { useApp } from '@renderer/store'
import { BAR_BANDS, clock, type Voice } from './useVoice'

/** Listening (the clock and a meter) or transcribing; nothing otherwise. */
export function VoiceStatus({ voice }: { voice: Voice }) {
  if (voice.phase === 'recording') {
    return (
      <span className="voice-status">
        <span className="voice-rec-dot" />
        <span className="voice-time">{clock(voice.elapsed)}</span>
        <span className="voice-meter" aria-hidden>
          {BAR_BANDS.map((band, i) => (
            <span
              key={band}
              className="voice-bar"
              ref={(el) => {
                voice.bars.current[i] = el
              }}
            />
          ))}
        </span>
      </span>
    )
  }
  if (voice.phase === 'transcribing') {
    return (
      <span className="voice-status">
        <Loader2 size={13} className="spin" />
        Transcribing…
      </span>
    )
  }
  return null
}

/** What went wrong, with what can be done about it. */
export function VoiceNotice({ voice }: { voice: Voice }) {
  const openSettings = useApp((s) => s.openSettings)
  const notice = voice.notice
  if (!notice) return null
  return (
    <div className="voice-notice" role="alert">
      <AlertCircle size={14} className="voice-notice-icon" />
      <span className="voice-notice-text">{notice.message}</span>
      {notice.settings && (
        <button className="btn voice-notice-action" onClick={openSettings}>
          <Settings size={12} />
          Settings
        </button>
      )}
      {notice.microphone && (
        <button className="btn voice-notice-action" onClick={() => void window.api.voice.openMicrophoneSettings()}>
          Open System Settings
        </button>
      )}
      {notice.retry && (
        <button className="btn voice-notice-action" onClick={voice.retry} title="Send the same recording again">
          <RotateCcw size={12} />
          Retry
        </button>
      )}
      <button className="voice-icon-btn" onClick={voice.dismiss} title="Dismiss" aria-label="Dismiss">
        <X size={13} />
      </button>
    </div>
  )
}
