// Entry point: wire the app to the real world and listen.

import { createApp } from './app.js';

let app;
try {
  app = createApp({ fetch: globalThis.fetch, env: process.env, now: Date.now });
} catch (err) {
  console.error(`Can't start: ${err.message}`);
  process.exit(1);
}

const port = process.env.PORT || 3000;

// No host argument: Node then listens on every interface, IPv6 included,
// which is how Laravel Cloud probes the app.
const server = app.listen(port, (err) => {
  if (err) {
    console.error(`Can't listen on port ${port}: ${err.message}`);
    process.exit(1);
  }
  console.log(`Nodus feedback server listening on port ${port}.`);
});

// Let in-flight reports finish on shutdown, but don't outstay the platform's
// grace period; unref() keeps the timer from holding the process open.
process.on('SIGTERM', () => {
  server.close(() => process.exit(0));
  setTimeout(() => process.exit(1), 25_000).unref();
});
