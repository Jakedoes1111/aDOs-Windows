# The Browser focus contract (F1 to F10)

Every change to the Browser keeps these rules. `scripts/focus-matrix.sh enter leave click overlays` checks them on a virtual display (Linux only). The rules apply on Windows too; the WebView2 mapping is in `windows-port.md` §2.

F1. **One keyboard owner, and it is visible.** The module whose rail icon and tab are marked active is the one that receives the next key. Never two carets, never none.
F2. **Entering the Browser** (Ctrl+number, rail, palette, tab click, layout switch, `browser_open`, a link opened from another module): focus goes to where it last was inside the Browser, page or address bar. First time: the page if one is loaded, the address bar if the page is blank.
F3. **A click in the page lands the first time** (link, field, button), whatever had focus before, and makes the Browser the active module.
F4. **Leaving the Browser**: the next key goes to the target module. The page receives nothing after the chord, including no stray modifier keydown/keyup. Text typed straight after the chord is not lost and not split between two modules.
F5. **Overlays** (palette, capture card, settings, layout menu, key editor, assistant card, comment card): on close, focus returns to exactly where it was, inside the page included.
F6. **A page can never take the keyboard** while another module or an overlay owns it (autofocus, `element.focus()`, a load finishing, a new window).
F7. **The OS window losing and regaining focus**, and minimise/restore, put the keyboard back where it was.
F8. **Launch** restores the last active module and its focus. The address bar is selected on launch only when the page is blank.
F9. **aDOs chords always win** over the page, and the engine's own window shortcuts (new tab, new window, close tab, reopen tab, devtools, print, quit...) never reach the engine.
F10. **Esc ladder inside the Browser**: page → address bar is the existing rule, keep it. Nothing else may leave the owner stuck in the page with no key that gets out.
