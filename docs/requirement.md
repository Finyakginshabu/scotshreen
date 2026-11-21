# Requirements: Scotshreen

A very lightweight background app for **long screenshots**. You select an area of the screen, capture it, scroll,
capture again, and the frames are joined into one tall image.

---

## 1. Goals

- Capture content that is taller than the screen (web pages, chats, documents) as one image.
- Stay very lightweight: small download, near-zero CPU and low RAM while idle, instant open.
- Work on **macOS and Windows**.

## 2. Non-goals (v1)

- No annotation tools (pen, arrow, text and so on).
- No screen recording or GIF.
- No live preview of the stitched result.
- No auto-capture while scrolling.
- No settings window. Defaults are hard-coded (the tray's "Open at login" toggle is the only option).
- No cloud upload or sharing.

---

## 3. User flow

1. Scotshreen runs in the background with a menu bar icon (macOS) or tray icon (Windows).
2. The user presses **`⌘⇧1`** (macOS) or **`Ctrl+Shift+1`** (Windows). A crosshair appears. The screen is never dimmed.
3. The user drags a rectangle to select the area. Releasing the mouse confirms it; no key press is needed.
4. The toolbar appears at the **top center** of the screen. The user can drag it anywhere. Until the first capture, the rectangle can still be resized from all edges and corners (it cannot be moved).
5. The user clicks **Capture**, which takes frame 1.
6. The user scrolls the content inside the rectangle and clicks **Capture** again for frame 2, 3, and so on.
7. Between frames, the user may change the rectangle's **height** but not its **width**.
8. The user clicks **Copy** or **Save**. The frames are joined into one long image and the app returns to the background.

---

## 4. Functional requirements

### 4.1 Background and hotkey
| ID | Requirement |
|---|---|
| R1 | The app runs in the background with a menu bar / tray icon and no window. |
| R2 | The hotkey opens capture mode: **macOS `⌘⇧1`**, **Windows `Ctrl+Shift+1`**. While an area is open, it discards that area and its frames and opens capture mode again (Space or the toolbar button takes the next frame). If another app owns the hotkey, the tray menu says so. |
| R3 | The tray menu has **Capture** (showing the hotkey), **Open folder**, **Open at login** and **Quit**. Tray **Capture** behaves like the hotkey. |
| R4 | `Esc` cancels capture mode at any time and discards all frames. With 2 or more frames it must be pressed twice (a message asks first). While a PNG is being saved, it waits for the save. |

### 4.2 Area selection
| ID | Requirement |
|---|---|
| R5 | Drag to draw a rectangle. Releasing the mouse confirms it and opens the toolbar. A click without a drag selects the window under the cursor. Nothing is dimmed: the screen keeps its original colours. |
| R6 | Before frame 1, the rectangle can be resized from all edges and corners, using 8 white dots (corners and edge middles) or the edges. It cannot be moved. After frame 1 only the top and bottom dots remain. |
| R7 | Readouts like macOS ⌘⇧4, in points: before pressing, the cursor position shows at the top-left of the cursor; on press it stays at the start point. While dragging, the width and height show at the bottom-right of the cursor. Both stay after release and update while resizing. After frame 1 the position readout shows the progress instead ("3 frames", total height). |
| R8 | The selection works on the monitor under the mouse. Spanning several monitors is not supported. |

### 4.3 Toolbar (menu)
| ID | Requirement |
|---|---|
| R9 | The toolbar appears at the **top center** of the active monitor when the rectangle is first drawn. |
| R10 | The toolbar can be **dragged anywhere** on screen by its handle. |
| R11 | Buttons: **Capture**, **Undo last frame**, **Copy**, **Save**, **Cancel**. |
| R12 | The toolbar shows the frame count, for example "3". |
| R13 | Keyboard shortcuts: `Space` = Capture, `Enter` = Copy, `⌘S` / `Ctrl+S` = Save, `⌘Z` / `Ctrl+Z` = Undo last frame, `Esc` = Cancel. They work while the app being captured has the keyboard. `Esc` works anywhere; the others only while the pointer is over the area or the toolbar, so typing in other apps still works. `M` (magnet) works only while the area is being drawn. Hovering a button shows a tooltip with its shortcut. |
| R14 | Copy and Save stay disabled until at least one frame has been captured. |
| R14a | **Layout: compact pill**, one row, about 40px tall. Order: drag handle ⠿ · **◉ Capture** (accent colour) · ↶ Undo with the frame count under it · magnet · **⧉ Copy** · **⤓ Save** · ✕ Cancel. Only Capture has a text label; the rest are icon-only (tooltips name them). Every button is exposed to screen readers (macOS). |
| R14b | Messages ("Copied", "Saved", the gap warning, errors) appear in a small line just under the pill, then fade out. Failures (capture, copy, save) are shown there too; after a failed Copy or Save the frames are kept. |
| R14c | **Style: liquid glass**, following the system light/dark mode. See section 8. |

```
╭───────────────────────────────────────────────────────╮
│ ⠿ │ ◉ Capture  ↶ 🧲 │ ⧉  ⤓ │ ✕ │
╰───────────────────────────────────────────────────────╯
          ⚠ No overlap found, frames may have a gap
```

### 4.4 Scrolling and multi-frame capture
| ID | Requirement |
|---|---|
| R15 | **Capture** takes the pixels inside the rectangle from the **live** screen. The border and toolbar are never included, even if the toolbar is dragged over the rectangle. |
| R16 | Mouse-wheel and trackpad scrolling inside the rectangle reach the app underneath, so the user can scroll without leaving capture mode. |
| R17 | The user can capture any number of frames. |
| R18 | After frame 1, the rectangle's **width and left/right position are locked**. Only the top and bottom edges can be dragged. |

### 4.5 Stitching (joining frames)
| ID | Requirement |
|---|---|
| R19 | Frames are joined top to bottom in capture order. |
| R20 | Overlapping content between frames is detected and removed, so nothing is duplicated. |
| R21 | If no overlap is found, frames are joined edge to edge, and the toolbar warns that the frames may have a gap. |
| R22 | Frames of different heights (because of R18) are handled. |

### 4.6 Output
| ID | Requirement |
|---|---|
| R23 | **Copy** joins the frames and copies the long image to the clipboard. |
| R24 | **Save** joins the frames, saves a PNG to `Pictures/Scotshreen/` named with a timestamp (never overwriting a file), and also copies it to the clipboard. The message says where it was saved. |
| R25 | A short confirmation is shown ("Copied" / "Saved"), then capture mode closes. |
| R26 | A single frame followed by Copy or Save works like a normal area screenshot. |

---

## 5. Non-functional requirements

| ID | Requirement | Target |
|---|---|---|
| N1 | Download / installed size | ≤ 5 MB (stretch: ≤ 2 MB) |
| N2 | RAM while idle in the background | ≤ 20 MB |
| N3 | CPU while idle | ~0% (no polling) |
| N4 | Time from hotkey to overlay showing | < 200 ms |
| N5 | Stitching time for 10 frames | < 1 s |
| N6 | Platforms | macOS 13+, Windows 10/11 |
| N7 | macOS Screen Recording permission | Asked for on first use, with a clear message if it's denied |

---

## 6. Known limitations and edge cases

- **Sticky headers and footers** (for example a website's fixed menu bar) repeat in every frame. Workaround: shrink the
  rectangle height (R18) after frame 1 to exclude them.
- **Animated content** (videos, carousels) can break overlap detection.
- **Scrolling too far** between frames (no overlap) leaves a gap. See R21.
- **Retina / HiDPI screens:** frames are captured at the screen's real pixel resolution.

---

## 7. Tech stack

**Pure Rust, no webview.** One small native binary per platform. Tauri is dropped: its webview is the main source of
weight and makes scroll pass-through (R16) harder.

| Need | Crate | Why |
|---|---|---|
| Event loop, windows | `winit` | Cross-platform windows with transparency and always-on-top |
| Drawing (dim, border, toolbar) | `softbuffer` + `tiny-skia` | CPU drawing, no GPU stack, small |
| Toolbar text and digits | `fontdue` + a small embedded font subset | Tiny, no system font lookup |
| Tray / menu bar icon and menu | `tray-icon` + `muda` | Small, native on both platforms |
| Global hotkey | `global-hotkey` | Native registration, no polling (N3) |
| Screen capture | `xcap` | Windows and macOS from one API |
| Clipboard | `arboard` | Copies images on both platforms |
| PNG encoding | `png` | Much smaller than the full `image` crate |
| Stitching | own code | Row-hash overlap search; no extra dependency |

**Window layout for capture mode** (this is what makes R16 work without platform hacks):
- While selecting, one full-screen dimmed window takes the drag.
- After the rectangle is set, that window is replaced by **four dim windows around the rectangle**, **thin edge
  windows** for the border and resize handles, and a **separate toolbar window**. The inside of the rectangle has no
  window over it, so scrolling and clicks go straight to the app underneath.
- The toolbar and border windows are hidden for the instant of each capture (R15).

**Build size:** release profile with `opt-level = "z"`, `lto = true`, `codegen-units = 1`, `strip = true`,
`panic = "abort"`. Expected binary: about 2–4 MB.

**Distribution:**
- macOS: `.app` in a `.dmg`, built with `cargo-bundle`. Unsigned for now (users right-click → Open).
- Windows: a single portable `.exe`, with no installer needed. Unsigned for now (SmartScreen → More info → Run anyway).

---

## 8. Toolbar look: liquid glass

The glass effect comes from the **operating system**, not from our own drawing. That keeps it free in size and CPU
(N1–N3), and it looks native. We draw only the icons, text and highlights on top.

| Platform | Effect used | Notes |
|---|---|---|
| macOS 26 (Tahoe) and later | Apple's real Liquid Glass (`NSGlassEffectView`) | Refraction and specular highlights from the system. |
| macOS 13–15 | Blur (`NSVisualEffectView`, HUD material) | Older macOS has no Liquid Glass, so it falls back to frosted blur. |
| Windows 11 | Acrylic blur backdrop (DWM) | Windows has no refraction effect. We add a light top-edge highlight and a thin bright border to imitate glass. |
| Windows 10 | Acrylic blur (older API) | Same as above, may be slightly less smooth when dragging. |
| Any platform with transparency effects turned off | Solid translucent fill | Respects the user's accessibility setting. |

- Light/dark: follows the system setting and switches live.
- Shape: fully rounded pill (corner radius = half the height).
- Crates: `window-vibrancy` for blur/acrylic, plus a small `objc2` binding for `NSGlassEffectView` on macOS 26.
- We do **not** write our own blur or refraction. It would cost CPU on every frame and is hard to do correctly
  for a window that sits above other apps.
