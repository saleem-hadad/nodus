//! Keeps the app up to date from GitHub releases: finding the latest release,
//! downloading the build for this machine and checking it against GitHub's
//! digest, then swapping it in for the running app once it quits.
//!
//! Ports `src/main/updater.ts`.
