import { execFileSync } from 'child_process'
import fs from 'fs'
import os from 'os'
import path from 'path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { gitDiff, gitStatus } from './git'

// The user's git config (signing, default branch, status options) must not
// change what these tests see. Hoisted, so it's set before env.ts reads the environment.
vi.hoisted(() => {
  process.env.GIT_CONFIG_GLOBAL = '/dev/null'
  process.env.GIT_CONFIG_NOSYSTEM = '1'
})

let root: string

const git = (cwd: string, ...args: string[]) =>
  execFileSync('git', ['-c', 'user.name=Test', '-c', 'user.email=test@example.com', ...args], {
    cwd,
    encoding: 'utf8'
  })

const write = (file: string, text: string) => {
  fs.mkdirSync(path.dirname(path.join(root, file)), { recursive: true })
  fs.writeFileSync(path.join(root, file), text)
}

beforeEach(() => {
  root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'nodus-git-')))
  git(root, 'init', '-q', '-b', 'main')
})

afterEach(() => {
  fs.rmSync(root, { recursive: true, force: true })
})

function commitFiles(files: Record<string, string>) {
  for (const [file, text] of Object.entries(files)) write(file, text)
  git(root, 'add', '-A')
  git(root, 'commit', '-q', '-m', 'files')
}

describe('gitStatus', () => {
  it('is null outside a repository', async () => {
    const plain = fs.mkdtempSync(path.join(os.tmpdir(), 'nodus-plain-'))
    try {
      expect(await gitStatus(plain)).toBeNull()
    } finally {
      fs.rmSync(plain, { recursive: true, force: true })
    }
  })

  it('reads a repository with no commits yet', async () => {
    write('first.txt', 'hi\n')
    expect(await gitStatus(root)).toEqual({
      branch: 'main',
      oid: null,
      ahead: 0,
      behind: 0,
      files: [{ path: 'first.txt', status: 'untracked' }]
    })
  })

  it('lists every kind of change, sorted by path', async () => {
    commitFiles({ 'a.txt': 'a\n', 'gone.txt': 'bye\n', 'old name.txt': 'same\n' })
    write('a.txt', 'a changed\n')
    write('c.txt', 'new\n')
    git(root, 'add', 'c.txt')
    fs.rmSync(path.join(root, 'gone.txt'))
    git(root, 'mv', 'old name.txt', 'new name.txt')
    write('dir/un tracked.txt', 'loose\n')

    const status = await gitStatus(root)
    expect(status?.branch).toBe('main')
    expect(status?.oid).toMatch(/^[0-9a-f]{40}$/)
    expect(status?.files).toEqual([
      { path: 'a.txt', status: 'modified' },
      { path: 'c.txt', status: 'added' },
      { path: 'dir/un tracked.txt', status: 'untracked' },
      { path: 'gone.txt', status: 'deleted' },
      { path: 'new name.txt', oldPath: 'old name.txt', status: 'renamed' }
    ])
  })

  it('counts commits ahead of the upstream', async () => {
    commitFiles({ 'a.txt': 'a\n' })
    const clone = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'nodus-clone-')))
    try {
      git(clone, 'clone', '-q', root, '.')
      fs.writeFileSync(path.join(clone, 'b.txt'), 'b\n')
      git(clone, 'add', 'b.txt')
      git(clone, 'commit', '-q', '-m', 'b')
      expect(await gitStatus(clone)).toMatchObject({ branch: 'main', upstream: 'origin/main', ahead: 1, behind: 0 })
    } finally {
      fs.rmSync(clone, { recursive: true, force: true })
    }
  })

  it('reads no branch on a detached head', async () => {
    commitFiles({ 'a.txt': 'a\n' })
    git(root, 'checkout', '-q', '--detach')
    expect((await gitStatus(root))?.branch).toBeNull()
  })
})

describe('gitDiff', () => {
  it('diffs a changed file against HEAD', async () => {
    commitFiles({ 'a.txt': 'one\ntwo\n' })
    write('a.txt', 'one\nthree\n')
    const { patch } = await gitDiff(root, { path: 'a.txt', status: 'modified' })
    expect(patch).toContain('-two\n+three\n')
  })

  it('diffs an untracked file as all new', async () => {
    commitFiles({ 'a.txt': 'a\n' })
    write('new.txt', 'fresh\n')
    const { patch } = await gitDiff(root, { path: 'new.txt', status: 'untracked' })
    expect(patch).toContain('+fresh\n')
  })

  it('diffs a staged file in a repository with no commits', async () => {
    write('first.txt', 'hi\n')
    git(root, 'add', 'first.txt')
    const { patch } = await gitDiff(root, { path: 'first.txt', status: 'added' })
    expect(patch).toContain('+hi\n')
  })

  it('takes paths from the repository root when the project is a folder inside it', async () => {
    commitFiles({ 'app/a.txt': 'one\n' })
    write('app/a.txt', 'two\n')
    const { patch } = await gitDiff(path.join(root, 'app'), { path: 'app/a.txt', status: 'modified' })
    expect(patch).toContain('+two\n')
  })

  it('says when a file is binary', async () => {
    commitFiles({ 'img.bin': 'a\0b' })
    write('img.bin', 'c\0d')
    expect(await gitDiff(root, { path: 'img.bin', status: 'modified' })).toEqual({ patch: '', binary: true })
  })

  it('refuses paths outside the repository', async () => {
    commitFiles({ 'a.txt': 'a\n' })
    for (const file of [
      { path: '../outside.txt', status: 'untracked' as const },
      { path: '/etc/hosts', status: 'untracked' as const },
      { path: 'a.txt', oldPath: '../../etc/passwd', status: 'renamed' as const },
      { path: '.', status: 'modified' as const }
    ]) {
      await expect(gitDiff(root, file)).rejects.toThrow('Not in the repository')
    }
  })

  it('fails outside a repository', async () => {
    const plain = fs.mkdtempSync(path.join(os.tmpdir(), 'nodus-plain-'))
    try {
      await expect(gitDiff(plain, { path: 'a.txt', status: 'modified' })).rejects.toThrow('Not a git repository')
    } finally {
      fs.rmSync(plain, { recursive: true, force: true })
    }
  })
})
