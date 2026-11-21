# Build plan: Scotshreen

How to build what [requirement.md](requirement.md) describes. Each milestone ends with something you can run and check
before moving on.

---

## 0. Clean up

1. Quit the running SnapTool process.
2. Delete `src/`, `src-tauri/` and `README.md`. Keep `requirement.md` and `plan.md`.
3. Put cargo on the PATH (`~/.cargo/bin` in `~/.zshrc`) and confirm `cargo --version` and `rustc --version`.
4. `cargo init --name scotshreen` in this folder.

---

## 1. Project layout

```
scotshreen/
├─ Cargo.toml
├─ assets/
│  ├─ font.ttf            small subset: digits, Latin letters, a few symbols
│  └─ tray.png            menu bar / tray icon
└─ src/
   ├─ main.rs             event loop and app state machine
   ├─ tray.rs             tray icon and menu
   ├─ hotkey.rs           global hotkey registration
   ├─ overlay/
   │  ├─ select.rs        full-screen dim window for drawing the rectangle
   │  ├─ frame.rs         dim windows around the rectangle + edge/handle windows
   │  └─ toolbar.rs       the pill: layout, buttons, hover, drag, messages
   ├─ render.rs           tiny-skia helpers, icons, text
   ├─ capture.rs          grab the rectangle's pixels from the live screen
   ├─ stitch.rs           join frames, remove the overlap
   ├─ output.rs           clipboard, PNG save, open folder
   └─ platform/
      ├─ macos.rs         Liquid Glass, menu-bar-only app, permission check
      └─ windows.rs       acrylic, DPI awareness
```

**App states:**

```
Idle ──hotkey──▶ Selecting ──mouse up──▶ Framing ──Copy/Save──▶ Finishing ──▶ Idle
                    │                       │
                    └────────Esc────────────┴──────Esc / ✕──────────────────▶ Idle
```

- **Idle:** only the tray icon and hotkey exist. No windows, no timers.
- **Selecting:** one full-screen dim window on the monitor under the mouse. Drag, move and resize the rectangle.
- **Framing:** the rectangle is "open". Dim windows around it, edge windows for resizing, and the toolbar. Frames are
  captured here.
- **Finishing:** stitch, copy/save, show the message, then close all windows.

All windows are created when entering a state and destroyed when leaving it, so Idle uses as little memory as possible.

---

## 2. Milestones

### M1: Background app
- Event loop with `winit`. On macOS, hide the Dock icon (menu bar only).
- Tray icon and menu (Capture, Open folder, Quit) with `tray-icon` + `muda`.
- Global hotkey `⌘⇧1` / `Ctrl+Shift+1` with `global-hotkey`. For now it just logs.
- **Check:** the app sits in the tray, the hotkey fires, Quit works. Idle RAM is ≤ 20 MB and CPU is ~0% (N2, N3).

### M2: Select the area (Selecting state)
- Full-screen borderless, transparent, always-on-top window on the monitor under the mouse.
- Dim everything except the rectangle. Crosshair cursor.
- Drag to draw. Move by dragging inside. Resize from 8 handles. Show the size in pixels (R5–R8).
- `Esc` cancels (R4).
- **Check:** drawing feels instant, and it works on Retina and on a second monitor.

### M3: Open frame (Framing state)
- Replace the full-screen window with: 4 dim windows around the rectangle, thin edge windows (border + resize),
  nothing over the inside.
- **Check (the most important one):** scrolling a web page inside the rectangle works with a mouse wheel and a
  trackpad, on both macOS and Windows (R16). If this fails, the window plan needs rethinking before going further.

### M4: Toolbar
- Separate small window, top center of the monitor (R9), draggable by ⠿ (R10).
- Draw the compact pill with `tiny-skia`: icons as vector paths, text with `fontdue`.
- Hover and pressed states. Copy and Save disabled until frame 1 (R14).
- Keyboard shortcuts (R13). Message line under the pill (R14b).
- Plain translucent fill for now. Glass comes in M8.
- **Check:** every button and shortcut responds. The toolbar can be dragged anywhere.

### M5: Capture frames
- **Capture:** hide the toolbar and edge windows, wait one screen refresh, grab the monitor with `xcap`, crop to the
  rectangle at real pixel size, show the windows again (R15).
- Keep frames in a list. Undo removes the last one. The frame count updates (R12, R17).
- After frame 1, lock left/right edges and allow only top/bottom resizing (R18).
- **Check:** captured frames match what is on screen, with no border or toolbar in them, on Retina and on Windows at
  150% scaling.

### M6: Stitching
Joins frame A (everything so far) with the new frame B. Both have the same width.

1. Turn each row of pixels into a small fingerprint. Ignore the right-most ~24 px, because the scrollbar moves between
   frames and would break matching.
2. Try overlap sizes from large to small. For each one, compare the bottom rows of A with the top rows of B.
3. Accept the largest overlap where the rows match, allowing a few slightly different pixels per row (anti-aliasing,
   cursor blinking).
4. Skip overlaps made only of blank rows (plain white background), since those match anywhere.
5. Append B without its overlapping rows. If nothing matches, append the whole of B and flag the gap warning (R21).

- Works with frames of different heights (R22). Target: 10 frames in under 1 s (N5).
- **Check:** unit tests with generated images (known overlaps, no overlap, blank areas, different heights), then
  real pages: a long article, a chat, a PDF.

### M7: Output
- **Copy:** stitch and put the image on the clipboard with `arboard` (R23).
- **Save:** stitch, write a PNG with `png` to `Pictures/Scotshreen/scotshreen-YYYY-MM-DD-HHMMSS.png`, and copy (R24).
- Show "Copied" or "Saved", then close (R25). One frame works like a normal screenshot (R26).
- Tray menu → Open folder opens `Pictures/Scotshreen/`.
- **Check:** pasting into Preview / Paint / a chat app works. The file opens.

### M8: Liquid glass
- macOS 26 and later: put an `NSGlassEffectView` behind the toolbar content (small `objc2` binding).
- macOS 13–15: `NSVisualEffectView` blur through `window-vibrancy`.
- Windows: acrylic through `window-vibrancy`, plus a drawn top highlight and thin bright border.
- Follow system light/dark live. Use a solid fill when transparency is turned off in system settings.
- **Check:** looks right on light and dark backgrounds, in both modes, on each platform.

### M9: Platform details
- macOS: check for Screen Recording permission on first capture. If it's missing, show a message with a button
  that opens the right System Settings page (N7).
- Windows: per-monitor DPI awareness so coordinates and pixels line up on mixed-scaling monitors.
- Make sure the overlay appears above full-screen apps on macOS.

### M10: Release
- Release profile: `opt-level = "z"`, `lto = true`, `codegen-units = 1`, `strip = true`, `panic = "abort"`.
- macOS: `.app` and `.dmg` with `cargo-bundle`. Windows: single `.exe`, built on a Windows PC.
- **Check:** measure size (≤ 5 MB, N1), idle RAM, and hotkey-to-overlay time (< 200 ms, N4).

---

## 3. Risks

| Risk | Impact | Plan |
|---|---|---|
| Scrolling doesn't reach the app under the rectangle on some setup | Breaks the core feature | Tested first in M3, before the toolbar or capture. Fallback: let the user scroll, then press the hotkey or Space for the next frame. |
| Keyboard shortcuts need our window focused, but focus should stay with the page being scrolled | Shortcuts or scrolling may fight | The toolbar takes focus for keys. Mouse-wheel scrolling goes to the window under the cursor on both OSes even without focus, which is verified in M3. |
| Sticky headers repeat in every frame | Messy result | Known limitation. The user shrinks the height after frame 1 (R18). |
| `NSGlassEffectView` API details on macOS 26 | Glass may need adjusting | Isolated in `platform/macos.rs`, with the blur fallback always available. |
| No Windows PC to test on | Windows bugs found late | Need access to one by M3. |

---

## 4. Order of work

M0 → M1 → M2 → **M3 (scroll test)** → M4 → M5 → M6 → M7 → M8 → M9 → M10

M6 (stitching) is plain Rust with no windows, so it can be built and tested at any time in parallel.
