// Searches what was said and done in a board's sessions: the prompts, Claude's
// replies, and its tool calls (paths, commands, and what it wrote), its
// subagents' included. Tool output is left out: it's most of a transcript's
// bytes, and a file a session only read would match everything in it.
//
// Each transcript's searchable text is kept in memory, and like the metadata
// in discovery.ts it is read again only from where it left off when the file
// grows.
import fs from 'fs'
import path from 'path'
import type { SearchSnippet, SearchSnippetKind, SessionSearchHit } from '@shared/types'
import { queryTerms, matchRanges, termsPattern } from '@shared/search'
import {
  assistantParts,
  listAgentFiles,
  promptFull,
  readLines,
  realRepoPath,
  timeOf,
  toolName,
  transcriptPath,
  userText,
  type Json
} from './discovery'

const MAX_RESULTS = 50
const SNIPPETS = 3
const SNIPPET_CHARS = 160
/** How much of the snippet comes before the match. */
const SNIPPET_LEAD = 48

/** Tool input fields worth searching, paths first since they say what was touched. */
const PATH_FIELDS = ['file_path', 'notebook_path', 'path']
const TEXT_FIELDS = ['command', 'pattern', 'url', 'query', 'description', 'skill', 'prompt']
const WRITTEN_FIELDS = ['content', 'new_string', 'old_string']

const KIND_WEIGHT: Record<SearchSnippetKind, number> = { prompt: 1.3, tool: 1.1, reply: 1 }
const AGENT_WEIGHT = 0.8

interface Doc {
  kind: SearchSnippetKind
  tool?: string
  /** For a tool call: the file it acted on, and where in `text` the path ends. */
  file?: string
  pathEnd?: number
  agent?: boolean
  text: string
  lower: string
  at: number
}

interface FileIndex {
  size: number
  mtimeMs: number
  offset: number
  docs: Doc[]
}

const indexes = new Map<string, FileIndex>()

/** Tool results are most of a transcript and never searched, so their lines aren't even parsed. */
const notToolResult = (raw: string) => !raw.includes('"type":"tool_result"')

function doc(kind: SearchSnippetKind, text: string, at: number, extra?: Partial<Doc>): Doc {
  return { kind, text, lower: text.toLowerCase(), at, ...extra }
}

function toolDoc(use: Json, at: number, agent: boolean, roots: string[]): Doc | null {
  const input: Json = use.input && typeof use.input === 'object' ? use.input : {}
  const str = (key: string) => (typeof input[key] === 'string' && input[key].trim() ? (input[key] as string) : null)
  const relative = (p: string) => {
    const root = roots.find((r) => p.startsWith(r + path.sep))
    return root ? p.slice(root.length + 1) : p
  }
  const paths = PATH_FIELDS.map(str).filter((v): v is string => v !== null).map(relative)
  const rest = [...TEXT_FIELDS, ...WRITTEN_FIELDS].map(str).filter((v): v is string => v !== null)
  if (paths.length === 0 && rest.length === 0) return null
  const head = paths.join('\n')
  return doc('tool', [head, ...rest].filter(Boolean).join('\n'), at, {
    tool: toolName(String(use.name)),
    file: paths[0] ? path.basename(paths[0]) : undefined,
    pathEnd: head.length,
    agent: agent || undefined
  })
}

/** The searchable messages and tool calls in a line. A subagent's prompt is its session's tool call, so only its work counts. */
function lineDocs(line: Json, agent: boolean, roots: string[], out: Doc[]) {
  if (line.isSidechain && !agent) return
  const at = timeOf(line) ?? 0
  if (line.type === 'user' && !agent) {
    const u = userText(line)
    if (u) out.push(doc('prompt', promptFull(u.text), at))
  } else if (line.type === 'assistant') {
    const { text, uses } = assistantParts(line)
    if (text) out.push(doc('reply', text, at, { agent: agent || undefined }))
    for (const use of uses) {
      const d = toolDoc(use, at, agent, roots)
      if (d) out.push(d)
    }
  }
}

/** A transcript's searchable text, read from where it was left off when the file has grown. */
function indexFile(file: string, agent: boolean, roots: string[]): Doc[] {
  let stat: fs.Stats
  try {
    stat = fs.statSync(file)
  } catch {
    indexes.delete(file)
    return []
  }
  const cached = indexes.get(file)
  if (cached && cached.size === stat.size && cached.mtimeMs === stat.mtimeMs) return cached.docs

  let fd: number
  try {
    fd = fs.openSync(file, 'r')
  } catch {
    return cached?.docs ?? []
  }
  try {
    const grown = cached && stat.size > cached.size
    const idx: FileIndex = grown ? cached : { size: 0, mtimeMs: 0, offset: 0, docs: [] }
    idx.offset = readLines(fd, idx.offset, stat.size, false, (l) => lineDocs(l, agent, roots, idx.docs), notToolResult)
    idx.size = stat.size
    idx.mtimeMs = stat.mtimeMs
    indexes.set(file, idx)
    return idx.docs
  } finally {
    fs.closeSync(fd)
  }
}

/** Whether reading the file means parsing it, rather than answering from memory. */
function stale(file: string) {
  const cached = indexes.get(file)
  if (!cached) return true
  try {
    const stat = fs.statSync(file)
    return stat.size !== cached.size || stat.mtimeMs !== cached.mtimeMs
  } catch {
    return false
  }
}

/** The part of a match's text worth showing: a line around the first place it matched. */
function snippet(d: Doc, terms: string[], phrase: string | null): SearchSnippet {
  const find = (re: RegExp | null) => (re ? re.exec(d.text) : null)
  const m = (phrase ? find(termsPattern([phrase])) : null) ?? find(termsPattern(terms))
  const at = m?.index ?? 0
  const matchEnd = at + (m?.[0].length ?? 0)
  // A tool call matched in its path shows the path; matched in what it wrote,
  // it shows that and names the file beside it.
  const inPath = d.pathEnd !== undefined && at < d.pathEnd
  const base = d.kind === 'tool' && !inPath && d.pathEnd ? d.pathEnd + 1 : 0
  const limit = inPath ? d.pathEnd! : d.text.length
  let start = Math.max(base, at - SNIPPET_LEAD)
  // Start on a word.
  if (start > base) {
    const space = d.text.slice(start, at).search(/\s/)
    if (space >= 0) start += space + 1
  }
  let end = Math.min(limit, start + SNIPPET_CHARS)
  if (end < limit) {
    const space = d.text.slice(matchEnd, end).lastIndexOf(' ')
    if (space > 0) end = matchEnd + space
  }
  let text = d.text.slice(start, end).replace(/\s+/g, ' ').trim()
  if (start > base) text = `…${text}`
  if (end < limit) text = `${text}…`
  return {
    kind: d.kind,
    tool: d.tool,
    file: d.kind === 'tool' && !inPath ? d.file : undefined,
    agent: d.agent,
    text,
    ranges: matchRanges(text, terms),
    timestamp: d.at
  }
}

export interface SearchTarget {
  sessionId: string
  title: string
}

interface Scored {
  doc: Doc
  score: number
  all: boolean
}

/**
 * The sessions whose title and transcripts hold every word of the query,
 * best first. A session ranks by how well its best matches fit: the whole
 * phrase over all its words over some, and what the user asked or a tool
 * touched over what Claude said.
 */
export async function searchSessions(repoPath: string, targets: SearchTarget[], query: string): Promise<SessionSearchHit[]> {
  const terms = queryTerms(query)
  if (terms.length === 0) return []
  const normalized = query.toLowerCase().trim().replace(/\s+/g, ' ')
  const phrase = terms.length > 1 ? normalized : null
  const roots = [...new Set([repoPath, realRepoPath(repoPath)])]

  const scored: { hit: SessionSearchHit; score: number; latest: number }[] = []
  for (const t of targets) {
    const files = [
      { file: transcriptPath(repoPath, t.sessionId), agent: false },
      ...listAgentFiles(repoPath, t.sessionId).map((a) => ({ file: a.file, agent: true }))
    ]
    // Reading a transcript the first time takes a moment, so let other work in between.
    if (files.some((f) => stale(f.file))) await new Promise((resolve) => setImmediate(resolve))

    const covered = new Array<boolean>(terms.length).fill(false)
    const title = t.title.toLowerCase()
    terms.forEach((term, i) => (covered[i] = title.includes(term)))
    const titleWords = covered.filter(Boolean).length
    const titleScore = (phrase && title.includes(phrase)) || titleWords === terms.length ? 4 : titleWords / terms.length

    const hits: Scored[] = []
    for (const { file, agent } of files) {
      for (const d of indexFile(file, agent, roots)) {
        let found = 0
        for (let i = 0; i < terms.length; i++) {
          if (d.lower.includes(terms[i])) {
            found++
            covered[i] = true
          }
        }
        if (found === 0) continue
        const all = found === terms.length
        const fit = phrase && d.lower.includes(phrase) ? 3 : all ? 2 : found / terms.length
        hits.push({ doc: d, score: fit * KIND_WEIGHT[d.kind] * (d.agent ? AGENT_WEIGHT : 1), all })
      }
    }
    if (!covered.every(Boolean)) continue

    hits.sort((a, b) => b.score - a.score || b.doc.at - a.doc.at)
    const complete = hits.filter((h) => h.all).length
    const matches = complete || hits.length
    const best = hits.slice(0, 5).reduce((sum, h) => sum + h.score, 0)
    // The same command run twice reads the same; show it once.
    const snippets: SearchSnippet[] = []
    for (const h of hits) {
      if (snippets.length === SNIPPETS) break
      const s = snippet(h.doc, terms, phrase)
      if (!snippets.some((o) => o.text === s.text && o.tool === s.tool)) snippets.push(s)
    }
    scored.push({
      hit: { sessionId: t.sessionId, matches, snippets },
      score: titleScore + best + Math.log2(1 + matches) / 2,
      latest: hits.reduce((max, h) => Math.max(max, h.doc.at), 0)
    })
  }

  return scored
    .sort((a, b) => b.score - a.score || b.latest - a.latest)
    .slice(0, MAX_RESULTS)
    .map((s) => s.hit)
}

/** Lets go of what's been read, for tests. */
export function clearSearchIndex() {
  indexes.clear()
}
