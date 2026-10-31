import { useCallback, useEffect, useRef, useState, type RefObject } from 'react'
import type { TranscribeResult } from '@shared/types'
import { useApp } from '@renderer/store'
import { BANDS, startRecording, type Recording } from './recorder'

/** A recording stops by itself after this long. */
const MAX_MS = 5 * 60_000
/** Anything shorter is a double click, not a prompt. */
const MIN_MS = 600
/** Which band each meter bar shows: the loudest, lowest one in the middle. */
export const BAR_BANDS = [3, 1, 0, 2, 4].filter((band) => band < BANDS)

export type Phase = 'idle' | 'opening' | 'recording' | 'transcribing'

export interface Notice {
  message: string
  /** Offers Settings, for a missing or refused key. */
  settings?: boolean
  /** Offers System Settings, for a microphone the user turned down. */
  microphone?: boolean
  /** Offers to send the same recording again. */
  retry?: boolean
}

export const clock = (ms: number) => {
  const s = Math.floor(ms / 1000)
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

/** The microphone listens for one thing at a time: starting another recording cancels this one. */
let listening: (() => void) | null = null

interface Options {
  /** A field of the caller's own whose Esc cancels too; any other field's Esc is its own. */
  field?: RefObject<HTMLElement | null>
}

/**
 * Speaking a prompt: opening the microphone, the clock and meter while it
 * listens, and xAI's transcript, handed to `onText`. A mic button and what it
 * shows are the caller's.
 */
export function useVoice(projectId: string, onText: (text: string) => void, { field }: Options = {}) {
  const openSettings = useApp((s) => s.openSettings)
  const [phase, setPhase] = useState<Phase>('idle')
  const [notice, setNotice] = useState<Notice | null>(null)
  const [elapsed, setElapsed] = useState(0)

  const recording = useRef<Recording | null>(null)
  const startedAt = useRef(0)
  /** Bumped by every start and cancel, so an answer that arrives too late is dropped. */
  const run = useRef(0)
  /** A recording xAI couldn't transcribe, kept for Retry so nothing has to be said twice. */
  const failed = useRef<Blob | null>(null)
  /** Gets --level while recording, for a halo that swells with the voice. */
  const rootRef = useRef<HTMLDivElement>(null)
  const bars = useRef<(HTMLSpanElement | null)[]>([])
  const onTextRef = useRef(onText)
  onTextRef.current = onText

  const transcribe = useCallback(
    async (audio: Blob) => {
      const id = ++run.current
      setPhase('transcribing')
      setNotice(null)
      let result: TranscribeResult
      try {
        result = await window.api.voice.transcribe(projectId, new Uint8Array(await audio.arrayBuffer()), audio.type)
      } catch (err) {
        result = { error: err instanceof Error ? err.message : String(err) }
      }
      if (id !== run.current) return
      setPhase('idle')
      if ('error' in result) {
        failed.current = audio
        setNotice({ message: result.error, settings: result.keyProblem, retry: true })
        return
      }
      failed.current = null
      if (result.text) onTextRef.current(result.text)
      else setNotice({ message: 'Didn’t catch anything. Try again, a little closer to the mic.' })
    },
    [projectId]
  )

  const cancel = useCallback(() => {
    run.current++
    recording.current?.cancel()
    recording.current = null
    if (listening === cancel) listening = null
    setPhase('idle')
  }, [])

  const record = useCallback(async () => {
    if (listening !== cancel) listening?.()
    listening = cancel
    const id = ++run.current
    setNotice(null)
    setPhase('opening')
    const fail = (n: Notice | null) => {
      if (id !== run.current) return
      if (listening === cancel) listening = null
      setPhase('idle')
      setNotice(n)
    }
    if (!(await window.api.voice.keySource())) {
      fail(null)
      if (id === run.current) openSettings()
      return
    }
    if (!(await window.api.voice.microphone())) {
      return fail({ message: 'Claude Nodes isn’t allowed to use the microphone.', microphone: true })
    }
    let rec: Recording
    try {
      rec = await startRecording()
    } catch (err) {
      return fail({ message: `Couldn’t open the microphone: ${err instanceof Error ? err.message : String(err)}` })
    }
    // Cancelled while the microphone was opening.
    if (id !== run.current) return rec.cancel()
    recording.current = rec
    startedAt.current = Date.now()
    setElapsed(0)
    setPhase('recording')
  }, [openSettings, cancel])

  const stop = useCallback(async () => {
    const rec = recording.current
    if (!rec) return
    recording.current = null
    if (listening === cancel) listening = null
    const id = run.current
    const audio = await rec.stop()
    if (id !== run.current) return
    if (Date.now() - startedAt.current < MIN_MS) setPhase('idle')
    else void transcribe(audio)
  }, [transcribe, cancel])

  const retry = useCallback(() => {
    if (failed.current) void transcribe(failed.current)
  }, [transcribe])

  /** Forgets the notice, and the recording kept for Retry with it. */
  const dismiss = useCallback(() => {
    failed.current = null
    setNotice(null)
  }, [])

  // While recording: the clock, the meter, and the time limit.
  useEffect(() => {
    if (phase !== 'recording') return
    let frame = 0
    const draw = () => {
      const levels = recording.current?.levels()
      if (levels) {
        BAR_BANDS.forEach((band, i) => bars.current[i]?.style.setProperty('--level', levels[band].toFixed(3)))
        rootRef.current?.style.setProperty('--level', Math.max(...levels).toFixed(3))
      }
      frame = requestAnimationFrame(draw)
    }
    frame = requestAnimationFrame(draw)
    const ticker = window.setInterval(() => setElapsed(Date.now() - startedAt.current), 250)
    const limit = window.setTimeout(() => void stop(), MAX_MS)
    const root = rootRef.current
    return () => {
      cancelAnimationFrame(frame)
      window.clearInterval(ticker)
      window.clearTimeout(limit)
      root?.style.removeProperty('--level')
    }
  }, [phase, stop])

  // Esc stops listening (or waiting) and keeps nothing new; it comes before the board's own Esc.
  useEffect(() => {
    if (phase === 'idle') return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      const target = e.target as HTMLElement | null
      // Another field's Esc (a terminal's, say) is its own.
      if (target !== field?.current && target?.closest('input, textarea, [contenteditable="true"]')) return
      e.stopPropagation()
      cancel()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [phase, cancel, field])

  // A notice with nothing to act on goes away by itself.
  useEffect(() => {
    if (!notice || notice.settings || notice.microphone || notice.retry) return
    const t = window.setTimeout(() => setNotice((n) => (n === notice ? null : n)), 4200)
    return () => window.clearTimeout(t)
  }, [notice])

  // Leaving stops the microphone.
  useEffect(
    () => () => {
      run.current++
      recording.current?.cancel()
      if (listening === cancel) listening = null
    },
    [cancel]
  )

  return { phase, elapsed, notice, record, stop, cancel, retry, dismiss, rootRef, bars }
}

export type Voice = ReturnType<typeof useVoice>
