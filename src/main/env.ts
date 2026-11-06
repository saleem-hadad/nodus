// Resolves the user's login-shell environment and the `claude` binary.
//
// Apps launched from Finder/Dock get a minimal launchd environment (no PATH
// from ~/.zshrc etc.), so `claude` would not be found. We capture the full
// environment once from an interactive login shell at startup. Windows apps
// get the user's whole environment already, so there's nothing to capture.
import { execFile } from 'child_process'
import fs from 'fs'
import os from 'os'
import path from 'path'

const isWindows = process.platform === 'win32'

const MARK_START = '__NODUS_ENV_START__'
const MARK_END = '__NODUS_ENV_END__'

let baseEnv: Record<string, string> = toStringEnv(process.env)
let claudePath: string | null = null
let claudeVersion: string | null = null

function toStringEnv(env: NodeJS.ProcessEnv): Record<string, string> {
  const out: Record<string, string> = {}
  for (const [k, v] of Object.entries(env)) {
    if (typeof v !== 'string') continue
    // Windows calls it Path, and its names ignore case: one PATH, so lookups and children agree.
    if (isWindows && k.toUpperCase() === 'PATH') out.PATH ??= v
    else out[k] = v
  }
  return out
}

/** Session markers set by a parent Claude Code process; claude refuses to start nested. */
const STRIP_EXACT = new Set([
  'CLAUDECODE',
  'CLAUDE_CODE_ENTRYPOINT',
  'CLAUDE_CODE_EXECPATH',
  'CLAUDE_CODE_CHILD_SESSION',
  'CLAUDE_PID',
  'CLAUDE_EFFORT',
  'ELECTRON_RUN_AS_NODE'
])
const STRIP_PREFIXES = [
  'CLAUDE_CODE_SESSION',
  'CLAUDE_CODE_MESSAGING',
  'CLAUDE_CODE_SSE',
  'CLAUDE_AGENT_SDK',
  'NODUS_',
  'ELECTRON_'
]

function shouldStrip(key: string) {
  return STRIP_EXACT.has(key) || STRIP_PREFIXES.some((p) => key.startsWith(p))
}

function captureShellEnv(): Promise<Record<string, string> | null> {
  if (isWindows) return Promise.resolve(null)
  const shell = process.env.SHELL || '/bin/zsh'
  const script = `printf '%s' '${MARK_START}'; env -0; printf '%s' '${MARK_END}'`
  return new Promise((resolve) => {
    execFile(
      shell,
      ['-ilc', script],
      { timeout: 10_000, maxBuffer: 8 * 1024 * 1024, env: process.env },
      (_err, stdout) => {
        // Interactive shells may exit non-zero or print noise; the markers are what matter.
        const start = stdout?.indexOf(MARK_START) ?? -1
        const end = stdout?.lastIndexOf(MARK_END) ?? -1
        if (start < 0 || end < 0 || end <= start) return resolve(null)
        const body = stdout.slice(start + MARK_START.length, end)
        const env: Record<string, string> = {}
        for (const entry of body.split('\0')) {
          const eq = entry.indexOf('=')
          if (eq > 0) env[entry.slice(0, eq)] = entry.slice(eq + 1)
        }
        resolve(Object.keys(env).length ? env : null)
      }
    )
  })
}

/** Where Claude Code's installers put it, for a PATH that doesn't have them yet. */
function fallbackDirs(env: Record<string, string>): string[] {
  const home = os.homedir()
  if (isWindows) {
    return [
      // The native installer, WinGet and npm.
      path.join(home, '.local', 'bin'),
      ...(env.LOCALAPPDATA ? [path.join(env.LOCALAPPDATA, 'Microsoft', 'WinGet', 'Links')] : []),
      ...(env.APPDATA ? [path.join(env.APPDATA, 'npm')] : [])
    ]
  }
  return [
    path.join(home, '.local/bin'),
    path.join(home, '.claude/local'),
    '/opt/homebrew/bin',
    '/usr/local/bin',
    '/usr/bin',
    '/bin',
    '/usr/sbin',
    '/sbin'
  ]
}

function withFallbackPath(env: Record<string, string>) {
  const parts = (env.PATH || '').split(path.delimiter).filter(Boolean)
  for (const dir of fallbackDirs(env)) if (!parts.includes(dir)) parts.push(dir)
  env.PATH = parts.join(path.delimiter)
}

/** The first of `bins` found on the PATH; each name is looked for in every folder before the next. */
function findOnPath(bins: string[], pathVar: string): string | null {
  for (const bin of bins) {
    for (const dir of pathVar.split(path.delimiter)) {
      if (!dir) continue
      const candidate = path.join(dir, bin)
      try {
        fs.accessSync(candidate, fs.constants.X_OK)
        if (fs.statSync(candidate).isFile()) return candidate
      } catch {
        // not here
      }
    }
  }
  return null
}

/**
 * The claude.exe an npm shim (claude.cmd) starts. A batch file can't take the
 * JSON options sessions are started with, as cmd.exe reads its quotes and
 * ampersands, so sessions run the program it points at instead.
 */
export function exeBehindShim(shim: string, text: string): string | null {
  const target = /"%~?dp0%?\\([^"]+?\.exe)"/i.exec(text)?.[1]
  // Not node.exe: a shim that runs a script has no program of its own to start.
  if (!target || /(^|\\)node\.exe$/i.test(target)) return null
  return path.win32.join(path.win32.dirname(shim), target)
}

/** Claude Code's own program, preferring the native installer's claude.exe over an npm shim. */
function findClaude(pathVar: string): string | null {
  if (!isWindows) return findOnPath(['claude'], pathVar)
  const found = findOnPath(['claude.exe', 'claude.cmd'], pathVar)
  if (!found || !/\.cmd$/i.test(found)) return found
  try {
    const exe = exeBehindShim(found, fs.readFileSync(found, 'utf8'))
    return exe && fs.existsSync(exe) ? exe : null
  } catch {
    return null
  }
}

function readVersion(bin: string, env: Record<string, string>): Promise<string | null> {
  return new Promise((resolve) => {
    execFile(bin, ['--version'], { timeout: 10_000, env, windowsHide: true }, (err, stdout) => {
      if (err) return resolve(null)
      resolve(stdout.trim().split(/\r?\n/)[0] || null)
    })
  })
}

export async function initEnv() {
  const captured = await captureShellEnv()
  const env = captured ? { ...toStringEnv(process.env), ...captured } : toStringEnv(process.env)
  withFallbackPath(env)
  baseEnv = env
  claudePath = findClaude(env.PATH)
  claudeVersion = claudePath ? await readVersion(claudePath, spawnEnv()) : null
}

/** Environment for spawned claude processes: the user's shell env minus nesting markers. */
export function spawnEnv(extra: Record<string, string> = {}): Record<string, string> {
  const env: Record<string, string> = {}
  for (const [k, v] of Object.entries(baseEnv)) if (!shouldStrip(k)) env[k] = v
  env.TERM = 'xterm-256color'
  env.COLORTERM = 'truecolor'
  if (!env.LANG) env.LANG = 'en_US.UTF-8'
  return { ...env, ...extra }
}

/** A variable from the user's login-shell environment. */
export function envVar(name: string): string | undefined {
  return baseEnv[name]
}

/** The user's login shell. On Windows, PowerShell: PowerShell 7 when it's installed. */
export function userShell(): string {
  if (isWindows) {
    const systemRoot = baseEnv.SystemRoot || baseEnv.SYSTEMROOT || 'C:\\Windows'
    return (
      findOnPath(['pwsh.exe'], baseEnv.PATH ?? '') ??
      path.join(systemRoot, 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe')
    )
  }
  const shell = baseEnv.SHELL || process.env.SHELL
  return shell && path.isAbsolute(shell) ? shell : '/bin/zsh'
}

/** A terminal card's shell: a login shell, as Terminal.app starts one, or PowerShell on Windows. */
export function terminalShell(): { file: string; args: string[] } {
  return { file: userShell(), args: isWindows ? ['-NoLogo'] : ['-l'] }
}

/**
 * Runs one command line in the user's shell. PowerShell is told to print
 * UTF-8, which Windows PowerShell otherwise doesn't when its output is piped.
 */
export function shellCommand(run: string): { file: string; args: string[] } {
  if (!isWindows) return { file: userShell(), args: ['-c', run] }
  const utf8 = '[Console]::OutputEncoding = [Text.Encoding]::UTF8; '
  return { file: userShell(), args: ['-NoProfile', '-NonInteractive', '-Command', utf8 + run] }
}

/**
 * Git for Windows' bash, which Claude Code runs hooks in when it finds one;
 * null without it, or off Windows. Looked for the way Claude Code does: its
 * CLAUDE_CODE_GIT_BASH_PATH, then beside the git on the PATH, then where Git
 * installs by default.
 */
export function gitBash(): string | null {
  if (!isWindows) return null
  return (foundBash ??= { path: findGitBash() }).path
}

let foundBash: { path: string | null } | undefined

function findGitBash(): string | null {
  const set = baseEnv.CLAUDE_CODE_GIT_BASH_PATH
  if (set) return fs.existsSync(set) ? set : null
  const candidates: string[] = []
  const git = findOnPath(['git.exe'], baseEnv.PATH ?? '')
  // …\Git\cmd\git.exe, …\Git\bin\git.exe or …\Git\mingw64\bin\git.exe.
  if (git) {
    for (let dir = path.dirname(git), i = 0; i < 3; dir = path.dirname(dir), i++) {
      candidates.push(path.join(dir, 'bin', 'bash.exe'))
    }
  }
  const local = baseEnv.LOCALAPPDATA && path.join(baseEnv.LOCALAPPDATA, 'Programs')
  for (const root of [baseEnv.ProgramFiles, baseEnv['ProgramFiles(x86)'], local]) {
    if (root) candidates.push(path.join(root, 'Git', 'bin', 'bash.exe'))
  }
  return candidates.find((c) => fs.existsSync(c)) ?? null
}

export function getClaudePath() {
  return claudePath
}

export function getClaudeVersion() {
  return claudeVersion
}

export function requireClaude(): string {
  if (!claudePath) {
    const install = isWindows ? 'irm https://claude.ai/install.ps1 | iex, in PowerShell' : 'npm i -g @anthropic-ai/claude-code'
    throw new Error(`Claude Code CLI not found. Install it (${install}) and restart Nodus.`)
  }
  return claudePath
}
