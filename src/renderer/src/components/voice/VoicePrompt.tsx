import { useCallback, useEffect, useImperativeHandle, useRef, useState, type Ref } from 'react'
import { Loader2, Mic, Square, X } from 'lucide-react'
import clsx from 'clsx'
import { useVoice } from './useVoice'
import { VoiceNotice, VoiceStatus } from './VoiceParts'
import './voice.css'

export interface VoiceHandle {
  /** Starts recording, or stops and transcribes: the board's V key. */
  toggle(): void
}

/** A running session the mic speaks into, rather than starting a new one. */
export interface VoiceTarget {
  sessionId: string
  title: string
}

interface Props {
  projectId: string
  /** The session selected on the board, which what's said is typed into; null starts a new one. */
  target: VoiceTarget | null
  /** Starts a session on the reviewed prompt. */
  onStart: (text: string) => void
  /** Types what was said into a session's prompt, without sending it; false if the session is gone. */
  onTypeIn: (sessionId: string, text: string) => boolean
  ref?: Ref<VoiceHandle>
}

/**
 * The mic button in the board's bottom-right corner. Speak, stop, and what was
 * said opens for review; Enter starts a session on it. With a session selected,
 * it's typed into that session instead, to finish there.
 */
export function VoicePrompt({ projectId, target, onStart, onTypeIn, ref }: Props) {
  /** The transcript under review; null when there's none. */
  const [draft, setDraft] = useState<string | null>(null)
  /** Where what's being said goes: a session, or the review box when null. */
  const [into, setInto] = useState<VoiceTarget | null>(null)
  const textRef = useRef<HTMLTextAreaElement>(null)

  const voice = useVoice(
    projectId,
    (text) => {
      // A session archived meanwhile leaves it for review, so nothing said is lost.
      if (into && onTypeIn(into.sessionId, text)) return
      // Speaking again while reviewing adds to what's there.
      setDraft((d) => (d?.trim() ? `${d.trimEnd()} ${text}` : text))
    },
    { field: textRef }
  )
  const { phase, record, stop, cancel } = voice

  const speak = useCallback(
    (session: VoiceTarget | null) => {
      setInto(session)
      void record()
    },
    [record]
  )

  const discard = () => {
    voice.dismiss()
    setDraft(null)
  }

  const submit = () => {
    const text = draft?.trim()
    if (!text || phase !== 'idle') return
    onStart(text)
    voice.dismiss()
    setDraft(null)
  }

  useImperativeHandle(
    ref,
    () => ({
      toggle() {
        if (phase === 'recording') void stop()
        else if (phase === 'idle' && draft === null) speak(target)
      }
    }),
    [phase, draft, stop, speak, target]
  )

  // The review box takes the keyboard, with the caret after the last word, whenever it's ready for it.
  const reviewing = draft !== null
  useEffect(() => {
    const el = textRef.current
    if (!reviewing || phase !== 'idle' || !el) return
    el.focus()
    el.setSelectionRange(el.value.length, el.value.length)
  }, [reviewing, phase])

  const listening = phase === 'recording'
  const busy = phase !== 'idle'
  const status = <VoiceStatus voice={voice} />
  const showsStatus = phase === 'recording' || phase === 'transcribing'

  return (
    <div className={clsx('voice', listening && 'is-recording')} ref={voice.rootRef}>
      {reviewing ? (
        <div className="voice-review" role="group" aria-label="Spoken prompt">
          <textarea
            ref={textRef}
            className="voice-text"
            value={draft}
            readOnly={busy}
            placeholder="Say or type a prompt"
            aria-label="Prompt"
            spellCheck
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault()
                submit()
              } else if (e.key === 'Escape') {
                e.preventDefault()
                discard()
              }
            }}
          />
          <VoiceNotice voice={voice} />
          <div className="voice-review-foot">
            {busy ? (
              <>
                {listening && (
                  <button className="voice-stop" onClick={() => void stop()} title="Stop (V)" aria-label="Stop">
                    <Square size={9} fill="currentColor" strokeWidth={0} />
                  </button>
                )}
                {status}
                {phase !== 'opening' && (
                  <button className="voice-icon-btn" onClick={cancel} title="Cancel (Esc)" aria-label="Cancel">
                    <X size={13} />
                  </button>
                )}
              </>
            ) : (
              <button className="btn btn-ghost voice-more" onClick={() => speak(null)} title="Say more; it’s added to the end">
                <Mic size={14} />
                Keep talking
              </button>
            )}
            <span className="voice-spacer" />
            <button className="btn btn-ghost" onClick={discard} title="Discard (Esc)">
              Discard
            </button>
            <button
              className="btn btn-primary"
              onClick={submit}
              disabled={busy || !draft.trim()}
              title="Start a session on this prompt (Return)"
            >
              Start session <span className="voice-key">↩</span>
            </button>
          </div>
        </div>
      ) : (
        <div className="voice-dock">
          {voice.notice ? (
            <VoiceNotice voice={voice} />
          ) : (
            showsStatus && (
              <div className="voice-pill">
                {status}
                {into && (
                  <span className="voice-into" title={into.title}>
                    into <span className="voice-into-title">{into.title}</span>
                  </span>
                )}
                <button className="voice-icon-btn" onClick={cancel} title="Cancel (Esc)" aria-label="Cancel">
                  <X size={13} />
                </button>
              </div>
            )
          )}
          <button
            className={clsx('voice-button', listening && 'is-recording')}
            onClick={() => (listening ? void stop() : speak(target))}
            disabled={phase === 'opening' || phase === 'transcribing'}
            title={
              listening
                ? into
                  ? `Stop and type into “${into.title}” (V)`
                  : 'Stop and review (V)'
                : target
                  ? `Speak into “${target.title}” (V)`
                  : 'Speak a prompt (V)'
            }
            aria-label={listening ? 'Stop recording' : target ? `Speak into ${target.title}` : 'Speak a prompt'}
          >
            {listening ? (
              <Square size={16} fill="currentColor" strokeWidth={0} />
            ) : phase === 'transcribing' ? (
              <Loader2 size={22} className="spin" />
            ) : (
              <Mic size={22} strokeWidth={2} />
            )}
          </button>
        </div>
      )}
    </div>
  )
}
