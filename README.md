# Scotshreen

A small background app for long screenshots on macOS and Windows (Not yet). Select an area, capture it, scroll, capture
again, and Scotshreen joins the frames into one tall image with the overlap removed.

I designed it to be simple and easy to use while effectively meeting its core goals.

## How to use it

1. Press **`⌘⇧1`** (macOS) or **`Ctrl+Shift+1`** (Windows), or choose **Capture** from the tray menu.
2. Drag a rectangle over the content. Click without dragging to select the window under the cursor.
3. Click **Capture** (or press `Space`) to take the first frame.
4. Scroll the content inside the rectangle. Scrolling goes straight to the app underneath. Capture again, and repeat.
5. Click **Copy** (`Enter`) or **Save** (`⌘S` / `Ctrl+S`). The frames are joined and the result goes to the clipboard.
   Save also writes a PNG to `Pictures/Scotshreen/`.

Before the first frame you can resize the rectangle from any edge or corner. After it, the width is locked and only the
top and bottom edges move. That lets you trim a sticky header or footer out of later frames.

### Shortcuts

| Key | Action |
|---|---|
| `⌘⇧1` / `Ctrl+Shift+1` | Open capture mode (pressing it again starts over) |
| `Space` | Capture a frame |
| `⌘Z` / `Ctrl+Z` | Undo the last frame |
| `Enter` | Copy the joined image |
| `⌘S` / `Ctrl+S` | Save as PNG (and copy) |
| `M` | Magnet on/off while drawing: edges snap to window borders and other lines |
| `Esc` | Cancel (press twice if you already have 2 or more frames) |

All keys except `Esc` work only while the pointer is over the area or the toolbar, so typing in other apps still
works.

### Tray menu

**Capture**, **Open folder** (opens `Pictures/Scotshreen/`), **Open at login** and **Quit**.

## Building

You need a recent stable Rust toolchain (the crate uses edition 2024, so Rust 1.85 or newer).

```sh
cargo run              # debug build
cargo build --release  # optimised, size-focused build in target/release/scotshreen
cargo test             # unit tests, mostly for the stitching code
```

On macOS the first capture asks for **Screen Recording** permission. If it's denied, Scotshreen opens the right page in
System Settings. Allow it there and try again.

> **macOS SDK note:** [.cargo/config.toml](.cargo/config.toml) pins `SDKROOT` to the macOS 26.5 SDK, which works
> around a broken 27.0 SDK in the current Command Line Tools. If you don't have that SDK, or the tools have been
> fixed, change or remove the line.

Windows release builds have no console window. Build them on a Windows machine.

## How it works

- **Capture mode never covers the inside of the rectangle.** After you draw the area, the full-screen selection window
  is replaced by thin edge windows plus a separate toolbar window. Nothing sits over the content, so the mouse wheel and
  trackpad reach the app underneath.
- **Frames come from the live screen.** The border and toolbar are kept out of every capture, even when the toolbar is
  dragged over the area.
- **Stitching** ([src/stitch.rs](src/stitch.rs)) fingerprints each pixel row, searches for the largest overlap between
  the image so far and the new frame, and appends only the new rows. It ignores the scrollbar column, tolerates small
  pixel differences, and skips blank-only matches. If no overlap is found, the frame is appended edge to edge and the
  toolbar warns about a possible gap.
- **Toolbar glass:** macOS 26 uses the system Liquid Glass (`NSGlassEffectView`), and macOS 13–15 falls back to a
  blurred `NSVisualEffectView`. Windows currently draws a plain translucent pill.

## Project layout

```
src/
├─ main.rs          event loop and app state machine (Idle → Selecting → Framing → Finishing)
├─ tray.rs          tray icon and menu
├─ hotkey.rs        global hotkey and in-capture shortcuts
├─ capture.rs       grab the rectangle's pixels from the screen
├─ stitch.rs        join frames and remove overlap
├─ output.rs        clipboard and PNG save
├─ login.rs         "Open at login" (LaunchAgent on macOS, Run key on Windows)
├─ platform.rs      native helpers winit doesn't cover (glass, focus, capture exclusion…)
└─ overlay/         selection, frame edges, toolbar, magnet, labels, toasts
assets/             UI font (Inter), toolbar icon font, app icon
tools/              build_icon_font.py: rebuilds assets/icons.ttf from tools/icon-src/*.svg
docs/               requirement.md (what it should do), plan.md (how it was built)
```