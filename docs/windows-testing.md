# Windows manual test checklist

The Windows `.exe` is cross-built from Linux (see `.cargo/config.toml`) and only
smoke-tested under Wine / CI, so give it a pass on a **real Windows machine**
before shipping. Grab the binary from the **Windows build** GitHub Actions run
(Artifacts → `sudokah-windows-x86_64`) or build locally with `cargo win`.

## Launch
- [ ] Double-clicking `sudokah.exe` opens the app with **no console window**
      behind it (release build sets `windows_subsystem = "windows"`).
- [ ] No missing-DLL error dialog (the MSVC build should be self-contained).
- [ ] Window title shows `Sudokah` (and `Sudokah — MM:SS` once a puzzle runs).

## Layout / rendering
- [ ] Board is a square with square cells and fills the width.
- [ ] Resize tall → controls spread to fill; resize wide (landscape) → controls
      move to the right of the board; near-square still looks right.
- [ ] Try a high-DPI display / display-scaling (e.g. 150%): text and board scale
      cleanly, nothing clipped.
- [ ] Modern styling renders: accent-blue active mode chip, rounded button chips,
      pill toggle switches.

## Interaction
- [ ] Click a cell; type digits; arrow keys move the selection.
- [ ] Mode buttons (Digit / Corner / Center / Color) and Z/X/C/V hotkeys switch
      modes; corner/center pencil marks and colors apply.
- [ ] Undo / Redo (and the on-screen D-pad + Undo/Redo) work.
- [ ] Delete/Backspace and the trash button clear a cell.

## Toggles / flags (order: Clues · Show errors · Set givens)
- [ ] **Clues** overlays candidate marks without touching your own marks.
- [ ] **Show errors** reddens digits that don't match the solution.
- [ ] **Set givens** is **off by default**; turning it on lets you mark givens.

## Puzzles / actions
- [ ] Easy / Medium / Hard / Expert each generate a solvable board.
- [ ] Load… accepts a valid puzzle and rejects an invalid/non-unique one.
- [ ] Solve fills the board; New / Clear and Clear marks work; the confirm dialog
      appears when it would discard changes.
- [ ] Completing a puzzle shows the "Solved!" banner + time; 🏆 Best times opens
      and records/clears correctly.

## Persistence (the cross-platform risk area)
- [ ] Start a puzzle, toggle **Clues** and **Show errors** on, then **close and
      reopen**: the in-progress puzzle, the timer, and both preferences come back.
- [ ] Complete or clear the board, reopen: preferences still persist (no puzzle).
