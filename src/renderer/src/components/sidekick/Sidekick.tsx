import { useEffect, useId, useImperativeHandle, useLayoutEffect, useRef, useState, type ReactNode, type Ref } from 'react'
import clsx from 'clsx'
import { ArrowUp, CornerDownRight, LayoutDashboard, Loader2, Mic, Square, X } from 'lucide-react'
import type { FolderColor } from '@shared/types'
import { ColorSwatches } from '@renderer/components/ColorSwatches'
import { FOLDER_HUES } from '@renderer/components/FolderIcon'
import { NOTE_COLOR } from '@renderer/components/board/notes'
import { useVoice } from '@renderer/components/voice/useVoice'
import { VoiceNotice, VoiceStatus } from '@renderer/components/voice/VoiceParts'
import { matchCommands, takeCommand, type Target } from './compose'
import '@renderer/components/voice/voice.css'
import './sidekick.css'

export interface SidekickHandle {
  /** Starts listening, or stops and puts what was said in the field: the board's V key. */
  toggleVoice(): void
  /** Takes the keyboard, starting an empty field with `text`: the board's / key. */
  focus(text?: string): void
}

/** What the bar was asked for. */
export type Submission =
  | { kind: 'session'; text: string }
  | { kind: 'into'; sessionId: string; text: string }
  | { kind: 'view'; text: string }
  | { kind: 'note'; text: string; color: FolderColor }

interface Props {
  projectId: string
  /** The one Claude session selected, which what's sent goes into. */
  into: { sessionId: string; title: string } | null
  /** How many selected things a new session gets to read first. */
  contextCount: number
  /** Makes it; resolves false to keep the words in the field. */
  onSubmit: (submission: Submission) => Promise<boolean>
  /** Stacked above the bar, such as the selection's actions. */
  above?: ReactNode
  ref?: Ref<SidekickHandle>
}

const swatchColor = (color: FolderColor) =>
  `linear-gradient(${FOLDER_HUES[color].frontTop}, ${FOLDER_HUES[color].frontBottom})`

/**
 * The board's prompt bar, bottom centre, with the mic beside it. What's typed
 * or said starts a session; /view and /note make those instead. With one
 * session selected it's sent into that session, and with more selected, they
 * go to the new session as context.
 */
export function Sidekick({ projectId, into, contextCount, onSubmit, above, ref }: Props) {
  const [text, setText] = useState('')
  const [target, setTarget] = useState<Target | null>(null)
  const [color, setColor] = useState<FolderColor>(NOTE_COLOR)
  const [focused, setFocused] = useState(false)
  /** The highlighted command, while the slash menu is open. */
  const [active, setActive] = useState(0)
  /** Esc closed the slash menu; typing opens it again. */
  const [menuDismissed, setMenuDismissed] = useState(false)
  /** The selected session the user chose not to send into, starting a new session instead. */
  const [dropped, setDropped] = useState<string | null>(null)
  const [picking, setPicking] = useState(false)
  const [sending, setSending] = useState(false)
  const fieldRef = useRef<HTMLTextAreaElement>(null)
  const pickerRef = useRef<HTMLDivElement>(null)
  const swatchRef = useRef<HTMLButtonElement>(null)
  /** Put the caret after the last word once the text has rendered. */
  const caretToEnd = useRef(false)
  const menuId = useId()

  const voice = useVoice(
    projectId,
    (said) => {
      // Speaking again adds to what's there.
      setText((t) => (t.trim() ? `${t.trimEnd()} ${said}` : said))
      caretToEnd.current = true
    },
    { field: fieldRef }
  )
  const { phase } = voice
  const listening = phase === 'recording' || phase === 'transcribing'

  const sendsInto = !target && into && into.sessionId !== dropped ? into : null
  const commands = target || menuDismissed || !focused ? [] : matchCommands(text)
  const menuOpen = commands.length > 0
  const highlighted = Math.min(active, commands.length - 1)
  const open = focused || Boolean(text) || Boolean(target) || Boolean(sendsInto) || phase !== 'idle' || contextCount > 0

  // A new selection is sent into again.
  useEffect(() => setDropped(null), [into?.sessionId])

  useLayoutEffect(() => {
    const el = fieldRef.current
    if (!caretToEnd.current || !el || listening) return
    caretToEnd.current = false
    el.focus()
    el.setSelectionRange(el.value.length, el.value.length)
  }, [text, listening])

  // The colour picker closes on a click anywhere else.
  useEffect(() => {
    if (!picking) return
    const onDown = (e: PointerEvent) => {
      const hit = e.target as Node
      // Its own swatch toggles it.
      if (!pickerRef.current?.contains(hit) && !swatchRef.current?.contains(hit)) setPicking(false)
    }
    window.addEventListener('pointerdown', onDown, true)
    return () => window.removeEventListener('pointerdown', onDown, true)
  }, [picking])

  useImperativeHandle(
    ref,
    () => ({
      toggleVoice() {
        if (phase === 'recording') void voice.stop()
        else if (phase === 'idle') void voice.record()
      },
      focus(start) {
        if (start && !text && !target) {
          setText(start)
          caretToEnd.current = true
        }
        fieldRef.current?.focus()
      }
    }),
    [phase, voice, text, target]
  )

  const pick = (next: Target) => {
    setTarget(next)
    setText('')
    setActive(0)
  }

  const submit = async () => {
    const words = text.trim()
    if (!words || phase !== 'idle' || sending || menuOpen) return
    const submission: Submission =
      target === 'view'
        ? { kind: 'view', text: words }
        : target === 'note'
          ? { kind: 'note', text: words, color }
          : sendsInto
            ? { kind: 'into', sessionId: sendsInto.sessionId, text: words }
            : { kind: 'session', text: words }
    setSending(true)
    let done = false
    try {
      done = await onSubmit(submission)
    } finally {
      setSending(false)
    }
    if (!done) return
    setText('')
    setTarget(null)
    voice.dismiss()
  }

  const sendLabel =
    target === 'view'
      ? 'Create view'
      : target === 'note'
        ? 'Add note'
        : sendsInto
          ? `Send to “${sendsInto.title}”`
          : 'Start session'
  const placeholder =
    target === 'view'
      ? 'Describe the view, e.g. failing tests by file'
      : target === 'note'
        ? 'Write a note…'
        : sendsInto
          ? 'Reply to this session…'
          : contextCount === 1
            ? 'Ask Claude about this…'
            : contextCount > 1
              ? `Ask Claude about these ${contextCount}…`
              : 'Ask Claude to build, fix or explain…'

  const orbLabel =
    phase === 'recording'
      ? 'Stop and review (V)'
      : phase === 'transcribing'
        ? 'Transcribing…'
        : 'Speak a prompt (V)'

  return (
    <div className="sidekick" ref={voice.rootRef}>
      {above}
      <VoiceNotice voice={voice} />
      <div className="sidekick-row">
        <div
          className={clsx(
            'sidekick-pill',
            open && 'is-open',
            phase === 'recording' && 'is-recording',
            (target || sendsInto) && 'has-token'
          )}
          onMouseDown={(e) => {
            // The bar's background is the field's, too.
            if (e.target !== e.currentTarget || listening) return
            e.preventDefault()
            fieldRef.current?.focus()
          }}
        >
          {menuOpen && (
            <div className="sidekick-menu" id={menuId} role="listbox" aria-label="Make something else">
              {commands.map((c, i) => (
                <div
                  key={c.target}
                  id={`${menuId}-${c.target}`}
                  role="option"
                  aria-selected={i === highlighted}
                  className={clsx('sidekick-option', i === highlighted && 'is-active')}
                  // Keeps the keyboard in the field.
                  onMouseDown={(e) => e.preventDefault()}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => pick(c.target)}
                >
                  <span className="sidekick-option-icon">
                    {c.target === 'view' ? (
                      <LayoutDashboard size={15} className="sidekick-view-icon" />
                    ) : (
                      <span className="sidekick-note-chip" style={{ background: swatchColor(color) }} />
                    )}
                  </span>
                  <span className="sidekick-option-command">{c.command}</span>
                  <span className="sidekick-option-text">{c.description}</span>
                </div>
              ))}
            </div>
          )}

          {picking && (
            <div className="sidekick-picker" ref={pickerRef}>
              <ColorSwatches
                value={color}
                label="Note colour"
                onPick={(c) => {
                  setColor(c)
                  setPicking(false)
                  fieldRef.current?.focus()
                }}
              />
            </div>
          )}

          {target === 'view' && (
            <span className="sidekick-token is-view">
              <LayoutDashboard size={13} />
              <span className="sidekick-token-label">View</span>
              <TokenClear label="Start a session instead" onClear={() => setTarget(null)} />
            </span>
          )}
          {target === 'note' && (
            <span className="sidekick-token is-note">
              <button
                ref={swatchRef}
                className="sidekick-note-swatch"
                style={{ background: swatchColor(color) }}
                onClick={() => setPicking((p) => !p)}
                title="Note colour"
                aria-label={`Note colour: ${color}`}
                aria-expanded={picking}
              />
              <span className="sidekick-token-label">Note</span>
              <TokenClear label="Start a session instead" onClear={() => setTarget(null)} />
            </span>
          )}
          {sendsInto && (
            <span className="sidekick-token is-into" title={`Sent into “${sendsInto.title}”`}>
              <CornerDownRight size={13} />
              <span className="sidekick-token-label">{sendsInto.title}</span>
              <TokenClear label="Start a new session instead" onClear={() => setDropped(sendsInto.sessionId)} />
            </span>
          )}

          {listening && (
            <div className="sidekick-status">
              <VoiceStatus voice={voice} />
              <span className="sidekick-spacer" />
              <button className="voice-icon-btn" onClick={voice.cancel} title="Cancel (Esc)" aria-label="Cancel">
                <X size={13} />
              </button>
            </div>
          )}

          <textarea
            ref={fieldRef}
            className="sidekick-field"
            hidden={listening}
            rows={1}
            value={text}
            placeholder={placeholder}
            aria-label="Prompt"
            aria-autocomplete="list"
            aria-controls={menuOpen ? menuId : undefined}
            aria-activedescendant={menuOpen ? `${menuId}-${commands[highlighted].target}` : undefined}
            spellCheck
            onFocus={() => setFocused(true)}
            onBlur={() => {
              setFocused(false)
              setMenuDismissed(false)
            }}
            onChange={(e) => {
              const value = e.target.value
              const typed = target ? null : takeCommand(value)
              if (typed) {
                setTarget(typed.target)
                setText(typed.rest)
              } else {
                setText(value)
              }
              setActive(0)
              setMenuDismissed(false)
            }}
            onKeyDown={(e) => {
              if (e.nativeEvent.isComposing) return
              if (menuOpen && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
                e.preventDefault()
                const step = e.key === 'ArrowDown' ? 1 : commands.length - 1
                setActive((highlighted + step) % commands.length)
              } else if (menuOpen && (e.key === 'Enter' || e.key === 'Tab')) {
                e.preventDefault()
                pick(commands[highlighted].target)
              } else if (menuOpen && e.key === 'Escape') {
                e.preventDefault()
                setMenuDismissed(true)
              } else if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                void submit()
              } else if (e.key === 'Backspace' && !text && (target || sendsInto)) {
                e.preventDefault()
                if (target) setTarget(null)
                else if (sendsInto) setDropped(sendsInto.sessionId)
              } else if (e.key === 'Escape') {
                e.preventDefault()
                if (picking) setPicking(false)
                else e.currentTarget.blur()
              }
            }}
          />

          {!text && !target && !sendsInto && !contextCount && !listening && (
            <span className="sidekick-hint" aria-hidden>
              <kbd>/</kbd> view or note
            </span>
          )}

          <button
            className="sidekick-send"
            onClick={() => void submit()}
            disabled={!text.trim() || phase !== 'idle' || sending || menuOpen}
            title={`${sendLabel} (Return)`}
            aria-label={sendLabel}
          >
            {sending ? <Loader2 size={15} className="spin" /> : <ArrowUp size={16} strokeWidth={2.4} />}
          </button>
        </div>

        <button
          className={clsx('sidekick-orb', phase === 'recording' && 'is-recording')}
          onClick={() => (phase === 'recording' ? void voice.stop() : void voice.record())}
          disabled={phase === 'opening' || phase === 'transcribing'}
          title={orbLabel}
          aria-label={orbLabel}
        >
          {phase === 'recording' ? (
            <Square size={16} fill="currentColor" strokeWidth={0} />
          ) : phase === 'transcribing' ? (
            <Loader2 size={22} className="spin" />
          ) : (
            <Mic size={22} strokeWidth={2} />
          )}
        </button>
      </div>
    </div>
  )
}

function TokenClear({ label, onClear }: { label: string; onClear: () => void }) {
  return (
    <button className="sidekick-token-clear" onClick={onClear} title={label} aria-label={label}>
      <X size={11} strokeWidth={2.6} />
    </button>
  )
}
