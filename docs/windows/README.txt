aDOs for Windows - early preview
=================================

aDOs is one window for notes, tasks, a calendar and AI agent sessions, driven
from the keyboard. This is an early build of the Windows port. It is built and
started automatically on Windows (GitHub's windows-latest), but few people have
used it on a real PC yet. Expect rough edges.

Needs
-----
- Windows 11, 64-bit (x64). Windows 10 is not supported yet.
- Nothing else for the app itself: no installer, no runtime to add.
- Optional, for the assistant and Sessions: Node.js (which provides npx) and an
  agent you are already signed in to (pi, Claude Code or Codex).

Start it
--------
1. Unzip the folder anywhere, for example to Documents\aDOs.
2. Double-click ados.exe.
3. Windows may say "Windows protected your PC" because the app is not signed
   yet. Click "More info", then "Run anyway". It asks only the first time.

Starting ados.exe again while it runs brings the open window to the front.

What is in this build
---------------------
Works: Notes (Ctrl+2), Sessions (Ctrl+6), Tasks (Ctrl+7), Calendar (Ctrl+8),
the command palette (Ctrl+K), quick capture (Ctrl+Shift+K), the assistant
(Alt+Space), layouts, Settings (Ctrl+Alt+,) and the shortcut editor (Ctrl+,).

Not yet: the Terminal and the Browser show "Not in this build yet."
Agents cannot use aDOs's own tools yet (the MCP connection).

When the Browser arrives: web pages are drawn by Windows' own WebView2 in a
separate layer that always sits on top of aDOs. When a floating card (the
assistant, a comment card) opens over a page, aDOs first tries to cut a
card-shaped hole in the page so the rest of the page stays visible. If that
does not work on your PC, the page is hidden while the card is open and comes
back when it closes. Both are expected; neither loses anything.

Your data
---------
Everything aDOs saves is in %APPDATA%\ados (paste that into the Explorer
address bar). Notes are plain Markdown files in its "notes" folder.
To keep data somewhere else, set the environment variable ADOS_HOME to a
folder before starting aDOs.

Remove it
---------
Delete the unzipped folder. To remove your data too, delete %APPDATA%\ados.
