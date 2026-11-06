import { contextBridge, ipcRenderer, webUtils, type IpcRendererEvent } from 'electron'
import { IPC, type Api } from '@shared/api'

function subscribe<A extends unknown[]>(channel: string, cb: (...args: A) => void) {
  const listener = (_e: IpcRendererEvent, ...args: unknown[]) => cb(...(args as A))
  ipcRenderer.on(channel, listener)
  return () => {
    ipcRenderer.removeListener(channel, listener)
  }
}

const api: Api = {
  appInfo: () => ipcRenderer.invoke(IPC.appInfo),
  takeOpenRequest: () => ipcRenderer.invoke(IPC.appTakeOpenRequest),
  pathForFile: (file) => webUtils.getPathForFile(file),
  showMenu: (x, y) => ipcRenderer.invoke(IPC.appShowMenu, x, y),
  menuCommand: (command) => ipcRenderer.invoke(IPC.appMenuCommand, command),

  settings: {
    get: () => ipcRenderer.invoke(IPC.settingsGet),
    update: (patch) => ipcRenderer.invoke(IPC.settingsUpdate, patch)
  },

  projects: {
    list: () => ipcRenderer.invoke(IPC.projectsList),
    add: (repoPath) => ipcRenderer.invoke(IPC.projectsAdd, repoPath),
    update: (id, patch) => ipcRenderer.invoke(IPC.projectsUpdate, id, patch),
    remove: (id) => ipcRenderer.invoke(IPC.projectsRemove, id),
    revealInFinder: (id) => ipcRenderer.invoke(IPC.projectsReveal, id),
    stats: () => ipcRenderer.invoke(IPC.projectsStats)
  },

  groups: {
    list: () => ipcRenderer.invoke(IPC.groupsList),
    create: (name) => ipcRenderer.invoke(IPC.groupsCreate, name),
    update: (id, patch) => ipcRenderer.invoke(IPC.groupsUpdate, id, patch),
    remove: (id) => ipcRenderer.invoke(IPC.groupsRemove, id)
  },

  board: {
    load: (projectId) => ipcRenderer.invoke(IPC.boardLoad, projectId),
    saveNodes: (projectId, patches) => ipcRenderer.invoke(IPC.boardSaveNodes, projectId, patches),
    saveArchive: (projectId, rect) => ipcRenderer.invoke(IPC.boardSaveArchive, projectId, rect),
    saveViewport: (projectId, viewport) =>
      ipcRenderer.invoke(IPC.boardSaveViewport, projectId, viewport),
    removeNode: (projectId, sessionId) =>
      ipcRenderer.invoke(IPC.boardRemoveNode, projectId, sessionId),
    saveSection: (projectId, section) =>
      ipcRenderer.invoke(IPC.boardSaveSection, projectId, section),
    removeSection: (projectId, sectionId) =>
      ipcRenderer.invoke(IPC.boardRemoveSection, projectId, sectionId),
    nameSection: (projectId, sessionIds) =>
      ipcRenderer.invoke(IPC.boardNameSection, projectId, sessionIds),
    saveNote: (projectId, note) => ipcRenderer.invoke(IPC.boardSaveNote, projectId, note),
    removeNotes: (projectId, noteIds) => ipcRenderer.invoke(IPC.boardRemoveNotes, projectId, noteIds),
    saveInk: (projectId, strokes) => ipcRenderer.invoke(IPC.boardSaveInk, projectId, strokes)
  },

  sessions: {
    create: (projectId, opts) => ipcRenderer.invoke(IPC.sessionCreate, projectId, opts),
    open: (projectId, sessionId, cols, rows) =>
      ipcRenderer.invoke(IPC.sessionOpen, projectId, sessionId, cols, rows),
    // Keystrokes and resizes are fire-and-forget to keep typing latency low.
    write: (sessionId, data) => ipcRenderer.send(IPC.sessionWrite, sessionId, data),
    send: (projectId, sessionId, text) => ipcRenderer.invoke(IPC.sessionSend, projectId, sessionId, text),
    resize: (sessionId, cols, rows) => ipcRenderer.send(IPC.sessionResize, sessionId, cols, rows),
    kill: (sessionId) => ipcRenderer.invoke(IPC.sessionKill, sessionId),
    transcript: (projectId, sessionId) =>
      ipcRenderer.invoke(IPC.sessionTranscript, projectId, sessionId),
    summarize: (projectId, sessionIds, requestId, focus) =>
      ipcRenderer.invoke(IPC.sessionSummarize, projectId, sessionIds, requestId, focus),
    search: (projectId, query) => ipcRenderer.invoke(IPC.sessionSearch, projectId, query),
    agents: (projectId, sessionId) => ipcRenderer.invoke(IPC.sessionAgents, projectId, sessionId),
    agentTranscript: (projectId, sessionId, agentId) =>
      ipcRenderer.invoke(IPC.sessionAgentTranscript, projectId, sessionId, agentId)
  },

  views: {
    create: (projectId, opts) => ipcRenderer.invoke(IPC.viewsCreate, projectId, opts),
    save: (projectId, patch) => ipcRenderer.invoke(IPC.viewsSave, projectId, patch),
    remove: (projectId, viewId) => ipcRenderer.invoke(IPC.viewsRemove, projectId, viewId),
    call: (projectId, viewId, method, args) => ipcRenderer.invoke(IPC.viewsCall, projectId, viewId, method, args),
    report: (projectId, viewId, report) => ipcRenderer.send(IPC.viewsReport, projectId, viewId, report),
    approve: (projectId, viewId, name, allow) => ipcRenderer.invoke(IPC.viewsApprove, projectId, viewId, name, allow),
    refresh: (projectId, viewId) => ipcRenderer.invoke(IPC.viewsRefresh, projectId, viewId),
    ask: (projectId, viewId, prompt) => ipcRenderer.invoke(IPC.viewsAsk, projectId, viewId, prompt),
    brain: (projectId, viewId) => ipcRenderer.invoke(IPC.viewsBrain, projectId, viewId)
  },

  git: {
    status: (projectId) => ipcRenderer.invoke(IPC.gitStatus, projectId),
    diff: (projectId, file) => ipcRenderer.invoke(IPC.gitDiff, projectId, file)
  },

  actions: {
    global: () => ipcRenderer.invoke(IPC.actionsGlobal),
    save: (projectId, action) => ipcRenderer.invoke(IPC.actionsSave, projectId, action),
    remove: (projectId, actionId) => ipcRenderer.invoke(IPC.actionsRemove, projectId, actionId),
    draft: (projectId, description, global) =>
      ipcRenderer.invoke(IPC.actionsDraft, projectId, description, global)
  },

  voice: {
    keySource: () => ipcRenderer.invoke(IPC.voiceKeySource),
    setKey: (key) => ipcRenderer.invoke(IPC.voiceSetKey, key),
    microphone: () => ipcRenderer.invoke(IPC.voiceMicrophone),
    openMicrophoneSettings: () => ipcRenderer.invoke(IPC.voiceMicrophoneSettings),
    transcribe: (projectId, audio, mimeType) =>
      ipcRenderer.invoke(IPC.voiceTranscribe, projectId, audio, mimeType)
  },

  updates: {
    status: () => ipcRenderer.invoke(IPC.updatesStatus),
    check: () => ipcRenderer.invoke(IPC.updatesCheck),
    install: () => ipcRenderer.invoke(IPC.updatesInstall)
  },

  capture: {
    ready: () => ipcRenderer.invoke(IPC.captureReady),
    targets: () => ipcRenderer.invoke(IPC.captureTargets),
    guess: (text, attachments) => ipcRenderer.invoke(IPC.captureGuess, text, attachments),
    submit: (submission) => ipcRenderer.invoke(IPC.captureSubmit, submission),
    screenshot: () => ipcRenderer.invoke(IPC.captureScreenshot),
    files: (paths) => ipcRenderer.invoke(IPC.captureFiles, paths),
    clipboard: () => ipcRenderer.invoke(IPC.captureClipboard),
    hide: () => ipcRenderer.send(IPC.captureHide),
    setPassThrough: (through) => ipcRenderer.send(IPC.capturePassThrough, through),
    open: (request) => ipcRenderer.invoke(IPC.captureOpen, request),
    openSettings: () => ipcRenderer.invoke(IPC.captureSettings),
    shortcut: () => ipcRenderer.invoke(IPC.captureShortcut),
    allow: () => ipcRenderer.invoke(IPC.captureAllow),
    show: () => ipcRenderer.invoke(IPC.captureShow)
  },

  on: {
    ptyData: (cb) => subscribe(IPC.evPtyData, cb),
    ptyExit: (cb) => subscribe(IPC.evPtyExit, cb),
    status: (cb) => subscribe(IPC.evStatus, cb),
    agentStatus: (cb) => subscribe(IPC.evAgentStatus, cb),
    usage: (cb) => subscribe(IPC.evUsage, cb),
    boardChanged: (cb) => subscribe(IPC.evBoardChanged, cb),
    projectsChanged: (cb) => subscribe(IPC.evProjectsChanged, cb),
    summarizeProgress: (cb) => subscribe(IPC.evSummarizeProgress, cb),
    runFinished: (cb) => subscribe(IPC.evRunFinished, cb),
    menu: (cb) => subscribe(IPC.evMenu, cb),
    updateStatus: (cb) => subscribe(IPC.evUpdateStatus, cb),
    view: (cb) => subscribe(IPC.evView, cb),
    openCard: (cb) => subscribe(IPC.evOpenCard, cb),
    captureShown: (cb) => subscribe(IPC.evCaptureShown, cb),
    captureContext: (cb) => subscribe(IPC.evCaptureContext, cb)
  }
}

contextBridge.exposeInMainWorld('api', api)
