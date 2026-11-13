
(function () {
  var boot = __BOOT__;
  var seq = 0, pending = {}, listeners = {};
  var current = { data: boot.data, state: boot.state, results: boot.results || {} };
  function post(msg) { msg.nodus = 1; parent.postMessage(msg, '*'); }
  function call(method, args) {
    return new Promise(function (resolve, reject) {
      var id = ++seq;
      pending[id] = { resolve: resolve, reject: reject };
      post({ id: id, method: method, args: args || {} });
    });
  }
  function report(err) {
    var text = err && (err.stack || err.message) ? (err.stack || err.message) : String(err);
    post({ error: String(text).slice(0, 2000) });
  }
  function emit(event, payload) {
    (listeners[event] || []).slice().forEach(function (fn) {
      try { fn(payload); } catch (err) { report(err); }
    });
  }
  function whenReady(fn) {
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', fn, { once: true });
    else Promise.resolve().then(fn);
  }
  window.addEventListener('message', function (e) {
    if (e.source !== parent) return;
    var m = e.data;
    if (!m || m.nodus !== 1) return;
    if (m.id) {
      var p = pending[m.id];
      if (!p) return;
      delete pending[m.id];
      if (m.error) p.reject(new Error(m.error)); else p.resolve(m.result);
      return;
    }
    if (m.event === 'data') { current.data = m.payload; nodus.data = m.payload; }
    else if (m.event === 'state') { current.state = m.payload; nodus.state = m.payload; }
    else if (m.event === 'result' && m.payload) { current.results[m.payload.name] = m.payload; }
    if (m.event) emit(m.event, m.payload);
  });
  var nodus = {
    viewId: boot.viewId,
    title: boot.title,
    canStartSessions: Boolean(boot.startsSessions),
    data: current.data,
    state: current.state,
    results: current.results,
    on: function (event, fn) {
      (listeners[event] = listeners[event] || []).push(fn);
      if (event === 'data' || event === 'state') {
        whenReady(function () {
          if (current[event] !== undefined && (listeners[event] || []).indexOf(fn) >= 0) {
            try { fn(current[event]); } catch (err) { report(err); }
          }
        });
      }
      return function () {
        listeners[event] = (listeners[event] || []).filter(function (f) { return f !== fn; });
      };
    },
    setState: function (state) {
      current.state = state;
      nodus.state = state;
      return call('setState', { state: state });
    },
    run: function (name) { return call('run', { name: name }); },
    ask: function (prompt) { return call('ask', { prompt: prompt }); },
    sessions: function () { return call('sessions'); },
    notes: function () { return call('notes'); },
    views: function () { return call('views'); },
    view: function (id) { return call('view', { id: id }); },
    git: function () { return call('git'); },
    openSession: function (id) { return call('openSession', { id: id }); },
    startSession: function (prompt, options) {
      return call('startSession', { prompt: prompt, title: options && options.title });
    },
    openUrl: function (url) { return call('openUrl', { url: url }); },
    submit: function (answers) { return call('submit', { answers: answers }); }
  };
  window.nodus = nodus;
  window.addEventListener('error', function (e) { report(e.error || e.message); });
  window.addEventListener('unhandledrejection', function (e) { report(e.reason); });
  window.addEventListener('load', function () { post({ loaded: true }); });
  // Links open in the browser. The frame itself never leaves the view.
  document.addEventListener('click', function (e) {
    var a = e.target && e.target.closest ? e.target.closest('a[href]') : null;
    if (!a) return;
    var href = a.getAttribute('href') || '';
    if (href.charAt(0) === '#') return;
    e.preventDefault();
    if (/^https?:/i.test(a.href)) call('openUrl', { url: a.href });
  }, true);
  // Scrolling over the page scrolls it where something in it can scroll that
  // way. Otherwise the board gets the scroll, to pan as it would over any
  // card, and a pinch always zooms the board. A gesture stays with whichever
  // of the two it started on.
  function room(pos, max, d) { return d < 0 ? pos > 0 : pos < max - 1; }
  function scrolls(el, dx, dy) {
    var root = document.documentElement;
    for (; el && el !== root && el.nodeType === 1; el = el.parentElement) {
      var s = getComputedStyle(el);
      if (dy && /auto|scroll/.test(s.overflowY) && room(el.scrollTop, el.scrollHeight - el.clientHeight, dy)) return true;
      if (dx && /auto|scroll/.test(s.overflowX) && room(el.scrollLeft, el.scrollWidth - el.clientWidth, dx)) return true;
    }
    // The page itself. While the root's overflow is visible, the body's is the page's.
    var axis = dy ? 'overflowY' : 'overflowX';
    var overflow = getComputedStyle(root)[axis];
    if (overflow === 'visible' && document.body) overflow = getComputedStyle(document.body)[axis];
    if (/hidden|clip/.test(overflow)) return false;
    return dy
      ? room(root.scrollTop, root.scrollHeight - root.clientHeight, dy)
      : room(root.scrollLeft, root.scrollWidth - root.clientWidth, dx);
  }
  var toBoard = false, lastWheel = 0;
  window.addEventListener('wheel', function (e) {
    if (e.defaultPrevented || (!e.deltaX && !e.deltaY)) return;
    var now = Date.now();
    if (e.ctrlKey) toBoard = true;
    else if (now - lastWheel > 250) {
      var across = Math.abs(e.deltaX) > Math.abs(e.deltaY);
      toBoard = !scrolls(e.target, across ? e.deltaX : 0, across ? 0 : e.deltaY);
    }
    lastWheel = now;
    if (!toBoard) return;
    e.preventDefault();
    post({ wheel: {
      dx: e.deltaX, dy: e.deltaY, mode: e.deltaMode, x: e.clientX, y: e.clientY,
      ctrl: e.ctrlKey, shift: e.shiftKey, alt: e.altKey, meta: e.metaKey
    } });
  }, { passive: false });
})();
