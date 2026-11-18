// aDOs comment mode, page side. Injected through Eval only while the owner comments; the
// expression yields `window.__adosComment`, the one global it keeps. Leaving comment mode
// (`stop`) removes every listener and the outline; numbered pins stay until the page unloads.
(() => {
  if (window.__adosComment) return window.__adosComment;
  const BLUE = '#376BFA';
  const TEXT_CAP = 1000, HTML_CAP = 4000, CONTEXT = 200;
  let on = false, hover = null, picked = null, held = null, outline = null, pins = null;
  // While a pick waits for aDOs, the element under it stays outlined.
  let frozen = false;

  function box() {
    const el = document.createElement('div');
    el.setAttribute('data-ados-comment', '');
    el.style.cssText = 'position:fixed;pointer-events:none;z-index:2147483646;box-sizing:border-box;' +
      `border:2px solid ${BLUE};border-radius:3px;background:rgba(55,107,250,0.08);display:none;`;
    (document.body || document.documentElement).appendChild(el);
    return el;
  }
  function ours(el) { return !!(el && el.closest && el.closest('[data-ados-comment]')); }
  function show(el) {
    if (!outline) outline = box();
    if (!el) { outline.style.display = 'none'; return; }
    const r = el.getBoundingClientRect();
    Object.assign(outline.style, { display: '', left: r.left - 2 + 'px', top: r.top - 2 + 'px',
      width: r.width + 4 + 'px', height: r.height + 4 + 'px' });
  }

  function esc(s) { return window.CSS && CSS.escape ? CSS.escape(s) : s.replace(/[^\w-]/g, c => '\\' + c); }
  function unique(sel) { try { return document.querySelectorAll(sel).length === 1; } catch (e) { return false; } }
  // Shortest walk up to an element with a unique id (or the root), nth-of-type below it.
  function selector(el) {
    const parts = [];
    for (let node = el; node && node.nodeType === 1; node = node.parentElement) {
      if (node.id && unique('#' + esc(node.id))) { parts.unshift('#' + esc(node.id)); break; }
      let part = node.tagName.toLowerCase();
      const parent = node.parentElement;
      if (parent) {
        const same = Array.from(parent.children).filter(c => c.tagName === node.tagName);
        if (same.length > 1) part += `:nth-of-type(${same.indexOf(node) + 1})`;
      }
      parts.unshift(part);
      if (unique(parts.join(' > '))) break;
    }
    return parts.join(' > ');
  }
  function label(el) {
    const by = el.getAttribute('aria-labelledby');
    const named = by && document.getElementById(by);
    return (el.getAttribute('aria-label') || (named && named.textContent) || el.getAttribute('alt') ||
      el.getAttribute('title') || el.getAttribute('placeholder') ||
      (el.labels && el.labels[0] && el.labels[0].textContent) || '').trim().slice(0, 200);
  }
  const ROLES = { A: 'link', BUTTON: 'button', IMG: 'img', INPUT: 'textbox', TEXTAREA: 'textbox', SELECT: 'combobox',
    FORM: 'form', NAV: 'navigation', MAIN: 'main', H1: 'heading', H2: 'heading', H3: 'heading', H4: 'heading',
    UL: 'list', OL: 'list', LI: 'listitem', TABLE: 'table', P: 'paragraph' };
  function role(el) {
    if (el.getAttribute('role')) return el.getAttribute('role');
    if (el.tagName === 'INPUT' && /^(button|submit|reset)$/i.test(el.type)) return 'button';
    if (el.tagName === 'INPUT' && /^(checkbox|radio)$/i.test(el.type)) return el.type.toLowerCase();
    return ROLES[el.tagName] || '';
  }
  function rects(r) {
    const v = [r.left, r.top, r.width, r.height].map(Math.round);
    return { viewport: v, page: [Math.round(r.left + scrollX), Math.round(r.top + scrollY), v[2], v[3]] };
  }
  function element(el) {
    const text = (el.innerText || el.value || el.textContent || '').replace(/\s+\n/g, '\n').trim();
    const html = el.outerHTML || '';
    return Object.assign({ kind: 'element', selector: selector(el), tag: el.tagName.toLowerCase(), role: role(el),
      label: label(el), text: text.slice(0, TEXT_CAP), textCut: text.length > TEXT_CAP,
      html: html.slice(0, HTML_CAP), htmlCut: html.length > HTML_CAP }, rects(el.getBoundingClientRect()));
  }
  function selection() {
    const sel = getSelection();
    if (!sel || sel.isCollapsed || !sel.rangeCount || !sel.toString().trim()) return null;
    const range = sel.getRangeAt(0);
    const root = document.body || document.documentElement;
    const before = document.createRange(), after = document.createRange();
    before.setStart(root, 0); before.setEnd(range.startContainer, range.startOffset);
    after.setStart(range.endContainer, range.endOffset); after.setEnd(root, root.childNodes.length);
    const anchor = range.commonAncestorContainer.nodeType === 1 ? range.commonAncestorContainer
      : range.commonAncestorContainer.parentElement;
    // The range lies inside its common ancestor; some engines (Servo) report a far larger box.
    let r = range.getBoundingClientRect();
    if (anchor) {
      const a = anchor.getBoundingClientRect();
      const left = Math.max(r.left, a.left), top = Math.max(r.top, a.top);
      const right = Math.min(r.right, a.right), bottom = Math.min(r.bottom, a.bottom);
      r = right > left && bottom > top ? { left, top, width: right - left, height: bottom - top } : a;
    }
    const flat = s => s.replace(/\s+/g, ' ');
    return Object.assign({ kind: 'text', text: sel.toString(), before: flat(before.toString()).slice(-CONTEXT).trimStart(),
      after: flat(after.toString()).slice(0, CONTEXT).trimEnd(), selector: anchor ? selector(anchor) : '' }, rects(r));
  }
  function page() {
    return { title: document.title, url: location.href, dpr: devicePixelRatio, w: innerWidth, h: innerHeight };
  }
  function wrap(target) { return JSON.stringify(Object.assign(page(), { target })); }

  function pick(el) {
    if (!el || ours(el)) return;
    held = el;
    picked = element(el);
    frozen = true;
    show(el);
  }
  function move(e) { if (!frozen && !ours(e.target)) { hover = e.target; show(hover); } }
  // Every press inside the page belongs to comment mode: nothing reaches the page meanwhile.
  function block(e) {
    if (ours(e.target)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    if (e.type === 'click' && !frozen) pick(e.target);
  }
  function track() { if (frozen && held) show(held); else if (hover) show(hover); }
  const EVENTS = ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click', 'dblclick', 'auxclick', 'contextmenu'];

  function pinLayer() {
    if (pins && pins.isConnected) return pins;
    pins = document.createElement('div');
    pins.setAttribute('data-ados-comment', '');
    pins.style.cssText = 'position:absolute;left:0;top:0;width:0;height:0;z-index:2147483645;';
    document.documentElement.appendChild(pins);
    return pins;
  }
  function addPin(n, comment, at) {
    const pin = document.createElement('button');
    pin.type = 'button';
    pin.textContent = String(n);
    pin.title = comment;
    pin.setAttribute('aria-label', `Comment ${n}: ${comment}`);
    pin.style.cssText = 'position:absolute;box-sizing:border-box;min-width:22px;height:22px;padding:0 6px;' +
      `border:0;border-radius:11px;background:${BLUE};color:#fff;font:bold 12px/22px sans-serif;cursor:pointer;` +
      `left:${Math.max(0, at[0] - 11)}px;top:${Math.max(0, at[1] - 11)}px;`;
    const note = document.createElement('div');
    note.textContent = comment;
    note.style.cssText = 'display:none;position:absolute;left:0;top:26px;min-width:160px;max-width:320px;padding:8px 10px;' +
      'border:1px solid #25282D;border-radius:10px;background:#08090C;color:#D4D7DF;font:13px/1.4 sans-serif;' +
      'white-space:pre-wrap;text-align:left;box-shadow:0 4px 12px rgba(0,0,0,0.4);';
    pin.appendChild(note);
    pin.addEventListener('click', e => {
      e.preventDefault();
      e.stopPropagation();
      note.style.display = note.style.display === 'none' ? '' : 'none';
    });
    pinLayer().appendChild(pin);
  }

  window.__adosComment = {
    start() {
      if (on) return true;
      on = true; frozen = false; picked = null; held = null;
      addEventListener('mousemove', move, true);
      addEventListener('scroll', track, true);
      for (const name of EVENTS) addEventListener(name, block, true);
      return true;
    },
    stop() {
      on = false; frozen = false; picked = null; held = null; hover = null;
      removeEventListener('mousemove', move, true);
      removeEventListener('scroll', track, true);
      for (const name of EVENTS) removeEventListener(name, block, true);
      if (outline) { outline.remove(); outline = null; }
      return true;
    },
    // The picked element, once: aDOs asks after every page press.
    take() { const p = picked; picked = null; return p ? wrap(p) : ''; },
    // Target `i` of the last hint collect becomes the pick.
    hint(i) {
      const el = window.__adosHints && window.__adosHints.target ? window.__adosHints.target(i) : null;
      if (!el) return '';
      pick(el);
      el.scrollIntoView && el.scrollIntoView({ block: 'nearest' });
      return this.take();
    },
    // Pick again after a cancelled card.
    resume() { frozen = false; held = null; show(hover); return true; },
    selection() { const s = selection(); return s ? wrap(s) : ''; },
    page() { return JSON.stringify(Object.assign(page(), { text: (document.body && document.body.innerText || '').slice(0, 20000) })); },
    // Numbered markers at page coordinates [x, y]; `list` is [[n, comment, [x, y]], ...].
    pin(list) { for (const [n, comment, at] of list) addPin(n, comment, at); return true; },
  };
  return window.__adosComment;
})()
