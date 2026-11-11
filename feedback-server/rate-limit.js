// Rolling-window rate limits kept in memory. The app scales to zero, so a
// restart forgets every count; that's an acceptable trade for a feedback form
// with no database.

const HOUR_MS = 60 * 60 * 1000;

export function createRateLimiter({ now, perClient, overall, windowMs = HOUR_MS }) {
  const clients = new Map(); // client key -> ascending timestamps
  let everyone = [];

  // Only accepted reports are stored and `overall` caps them, so this sweep
  // touches a few dozen entries at most.
  function forgetOld() {
    const cutoff = now() - windowMs;
    everyone = everyone.filter((t) => t > cutoff);
    for (const [key, times] of clients) {
      const recent = times.filter((t) => t > cutoff);
      if (recent.length > 0) clients.set(key, recent);
      else clients.delete(key);
    }
  }

  // When the oldest entry leaves the window, one slot frees up.
  function secondsUntilFree(times) {
    return Math.max(1, Math.ceil((times[0] + windowMs - now()) / 1000));
  }

  return {
    // Returns null when `key` may send a report now; otherwise which limit it
    // hit and how many seconds to wait.
    check(key) {
      forgetOld();
      const mine = clients.get(key) ?? [];
      if (mine.length >= perClient) return { scope: 'client', retryAfter: secondsUntilFree(mine) };
      if (everyone.length >= overall) return { scope: 'everyone', retryAfter: secondsUntilFree(everyone) };
      return null;
    },

    record(key) {
      const t = now();
      everyone.push(t);
      clients.set(key, [...(clients.get(key) ?? []), t]);
    },
  };
}
