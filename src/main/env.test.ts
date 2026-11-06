import { describe, expect, it } from 'vitest'
import { exeBehindShim } from './env'

const NPM = 'C:\\Users\\me\\AppData\\Roaming\\npm'

describe('exeBehindShim', () => {
  it('finds the program an npm shim starts', () => {
    const shim = [
      '@ECHO off',
      'GOTO start',
      ':find_dp0',
      'SET dp0=%~dp0',
      'EXIT /b',
      ':start',
      'SETLOCAL',
      'CALL :find_dp0',
      '"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe"   %*',
      ''
    ].join('\r\n')
    expect(exeBehindShim(`${NPM}\\claude.cmd`, shim)).toBe(
      `${NPM}\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe`
    )
  })

  it('finds nothing in a shim that runs a script with Node', () => {
    const shim = [
      '@ECHO off',
      'SETLOCAL',
      'CALL :find_dp0',
      'IF EXIST "%dp0%\\node.exe" (',
      '  SET "_prog=%dp0%\\node.exe"',
      ') ELSE (',
      '  SET "_prog=node"',
      ')',
      'endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & "%_prog%"  "%dp0%\\node_modules\\@anthropic-ai\\claude-code\\cli.js" %*'
    ].join('\r\n')
    expect(exeBehindShim(`${NPM}\\claude.cmd`, shim)).toBeNull()
  })
})
