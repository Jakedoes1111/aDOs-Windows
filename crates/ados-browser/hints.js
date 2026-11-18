// aDOs link hints. Engine-neutral: injected on every page (Servo --userscripts, CDP
// addScriptToEvaluateOnNewDocument). `f` labels every visible clickable, `F` opens a link
// in place of the page (aDOs has one Browser panel). aDOs drives the same labels from
// outside for its window-wide hint mode through collect/show/typed/pick/hide.
(() => {
  if (window.__adosHints) return;
  const ALPHABET = 'sadjklewcmpgh';
  const SELECTOR = 'a[href],button,input:not([type=hidden]),select,textarea,[role=button],[role=link],' +
    '[onclick],summary,label[for],[tabindex]:not([tabindex="-1"])';
  const NAMED = { Escape: 'escape', Enter: 'enter', ' ': 'space', Tab: 'tab', Backspace: 'backspace',
    Delete: 'delete', ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right' };
  let cfg = { hints: ['f'], hintsNew: ['shift-f'], bg: 'rgba(196,181,156,0.95)', fg: 'rgb(11,10,7)',
    typed: 'rgb(143,131,113)', timeout: 5500 };
  let state = null;
  let hiddenAt = 0;
  let collected = [];

  // Shortest unique labels, as Vimium makes them: no label is a prefix of another.
  function labels(count) {
    if (count <= 0) return [];
    const hints = [''];
    let offset = 0;
    while (hints.length - offset < count || hints.length === 1) {
      const hint = hints[offset++];
      for (const ch of ALPHABET) hints.push(ch + hint);
    }
    return hints.slice(offset, offset + count).sort().map(h => h.split('').reverse().join(''));
  }

  function chord(e) {
    if (['Shift', 'Control', 'Alt', 'Meta'].includes(e.key)) return '';
    const key = e.key.length === 1 ? e.key.toLowerCase() : (NAMED[e.key] || e.key.toLowerCase());
    return (e.ctrlKey ? 'ctrl-' : '') + (e.altKey ? 'alt-' : '') + (e.shiftKey ? 'shift-' : '') +
      (e.metaKey ? 'super-' : '') + key;
  }

  function editable(el) {
    if (!el) return false;
    if (el.isContentEditable) return true;
    const tag = el.tagName;
    if (tag === 'TEXTAREA' || tag === 'SELECT') return true;
    return tag === 'INPUT' && !/^(button|submit|reset|checkbox|radio|image|range|color|file)$/i.test(el.type);
  }

  function visibleRect(el) {
    for (const r of el.getClientRects()) {
      if (r.width < 1 || r.height < 1) continue;
      if (r.bottom < 0 || r.right < 0 || r.top > innerHeight || r.left > innerWidth) continue;
      const style = getComputedStyle(el);
      if (style.visibility === 'hidden' || style.display === 'none') return null;
      const x = Math.min(Math.max(r.left + r.width / 2, 0), innerWidth - 1);
      const y = Math.min(Math.max(r.top + r.height / 2, 0), innerHeight - 1);
      const top = document.elementFromPoint ? document.elementFromPoint(x, y) : null;
      if (top && top !== el && !el.contains(top) && !top.contains(el) &&
          !(top.tagName === 'LABEL' && top.control === el)) continue;
      return r;
    }
    return null;
  }

  // Visible targets in document order; labels closer than 14 px to a kept one are dropped.
  function targets() {
    const out = [];
    for (const el of document.querySelectorAll(SELECTOR)) {
      const r = visibleRect(el);
      if (!r) continue;
      const x = Math.max(r.left, 0), y = Math.max(r.top, 0);
      if (out.some(t => Math.abs(t.x - x) < 14 && Math.abs(t.y - y) < 14)) continue;
      out.push({ el, x, y, w: r.width, h: r.height });
    }
    return out;
  }

  function layer() {
    const root = document.createElement('div');
    root.setAttribute('data-ados-hints', '');
    root.style.cssText = 'position:fixed;left:0;top:0;width:0;height:0;z-index:2147483647;pointer-events:none;';
    (document.body || document.documentElement).appendChild(root);
    return root;
  }

  // Each label sits just above its target's top-left corner (3 px overlap), so the link
  // text stays readable while choosing.
  function draw(items) {
    const root = layer();
    for (const item of items) {
      const span = document.createElement('span');
      span.style.cssText = 'position:fixed;box-sizing:border-box;height:14px;min-width:11px;padding:0 3px;' +
        'border-radius:4px;font-family:sans-serif;font-size:12px;font-weight:bold;line-height:14px;text-align:center;white-space:nowrap;' +
        `left:${Math.round(item.x)}px;top:${Math.max(0, Math.round(item.y) - 11)}px;background:${cfg.bg};color:${cfg.fg};`;
      root.appendChild(span);
      item.node = span;
    }
    return root;
  }

  function paint() {
    for (const item of state.items) {
      const match = item.label.startsWith(state.typed);
      item.node.style.display = match ? '' : 'none';
      if (!match) continue;
      item.node.textContent = '';
      const done = document.createElement('span');
      done.style.color = cfg.typed;
      done.textContent = state.typed;
      item.node.append(done, item.label.slice(state.typed.length));
    }
  }

  function arm() {
    clearTimeout(state.timer);
    state.timer = setTimeout(hide, cfg.timeout);
  }

  function start(newTab) {
    hide();
    const items = targets();
    if (!items.length) return;
    labels(items.length).forEach((label, i) => { items[i].label = label; });
    state = { items, typed: '', newTab, external: false, root: draw(items), timer: 0 };
    paint();
    arm();
  }

  function hide() {
    if (!state) return;
    clearTimeout(state.timer);
    state.root.remove();
    state = null;
    hiddenAt = Date.now();
  }

  // Deferred: Servo stalls when a key handler navigates synchronously.
  function activate(el, newTab) { setTimeout(() => activateNow(el, newTab), 0); }
  function activateNow(el, newTab) {
    if (newTab && el.href) { location.href = el.href; return; }
    if (editable(el)) { el.focus(); return; }
    if (el.tagName === 'LABEL' && el.control && editable(el.control)) { el.control.focus(); return; }
    if (el.focus) el.focus();
    el.click();
  }

  function type(key) {
    if (key === 'backspace') {
      state.typed = state.typed.slice(0, -1);
    } else if (key.length === 1 && ALPHABET.includes(key)) {
      state.typed += key;
    } else {
      hide();
      return;
    }
    const left = state.items.filter(i => i.label.startsWith(state.typed));
    if (!left.length) { hide(); return; }
    const exact = left.find(i => i.label === state.typed);
    if (exact && left.length === 1) {
      const { newTab } = state;
      hide();
      activate(exact.el, newTab);
      return;
    }
    paint();
    arm();
  }

  addEventListener('keydown', e => {
    const key = chord(e);
    if (!key) return;
    if (state && !state.external) {
      e.preventDefault();
      e.stopImmediatePropagation();
      type(key);
      return;
    }
    if (state || e.isComposing || editable(document.activeElement)) return;
    const newTab = cfg.hintsNew.includes(key);
    if (!newTab && !cfg.hints.includes(key)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    start(newTab);
  }, true);
  // Never leave labels behind.
  for (const name of ['mousedown', 'wheel', 'scroll', 'blur', 'pagehide']) {
    addEventListener(name, () => { if (state && !state.external) hide(); }, true);
  }

  window.__adosHints = {
    config(next) { Object.assign(cfg, next); },
    labels,
    // For aDOs: visible targets as CSS-pixel rectangles of the viewport.
    collect() {
      hide();
      collected = targets();
      return JSON.stringify({ dpr: devicePixelRatio, w: innerWidth, h: innerHeight,
        items: collected.map(t => [t.x, t.y, t.w, t.h]) });
    },
    // `list[i]` labels collected[i]; null leaves it out.
    show(list) {
      hide();
      const items = [];
      list.forEach((label, i) => { if (label && collected[i]) items.push(Object.assign(collected[i], { label })); });
      state = { items, typed: '', newTab: false, external: true, root: draw(items), timer: 0 };
      paint();
      arm();
    },
    typed(prefix) { if (state) { state.typed = prefix; paint(); arm(); } },
    pick(i) { hide(); if (collected[i]) activate(collected[i].el, false); collected = []; },
    hide() { hide(); collected = []; },
    // Comment mode (comment.js) picks target `i` instead of clicking it.
    target(i) { return collected[i] ? collected[i].el : null; },
    active() { return !!state; },
    // Esc reaches aDOs too: true if it was (or just was) cancelling page hints.
    cancel() {
      const was = !!state || Date.now() - hiddenAt < 500;
      hide();
      return was;
    },
  };
})();
