//! Every domain composed into one service, with no UI: async commands that
//! mirror `window.api` one to one, and a stream of the events the Electron
//! app sends on its `ev:*` channels. The UI talks to the rest of the app only
//! through this, and a headless Nodus can serve the same API.
//!
//! Ports `src/main/ipc.ts`, `src/shared/api.ts` and `src/preload/index.ts`,
//! and when to notify from `src/main/notify.ts`.
