# Media & Screenshot Capture Guide

This folder is reserved for real UI screenshots and short workflow GIFs/WebMs referenced from the main README. No placeholder graphics or mock renders are checked in — capture each item from a live build of **SFX de Babel** and replace the `TODO` lines below with Markdown image/video links as you go.

## Capture Setup

- **Window size:** Default `1220 × 760` centered (or `1280 × 800` for NLE side-by-side drag-and-drop).
- **Desktop backdrop:** Plain dark wallpaper behind the frameless window so the transparent glassmorphism and hairline borders read cleanly.
- **Video / GIF:** Record at `30 fps`, keep GIFs under `10 MB` (e.g., `ffmpeg` palettegen or ScreenToGif).
- **Suggested asset folder:** Save final files under `docs/media/` (e.g., `docs/media/envelope-drawing.gif`).

## Planned Captures

### 1. Drawing an Envelope Contour & Live Re-ranking (`envelope-drawing.gif`)
- **TODO:** Capture `docs/media/envelope-drawing.gif`
- **What to show:**
  1. Start with a search query (e.g. `whoosh` or `impact`) and results loaded.
  2. Draw a custom curve on the **Volume Envelope** canvas (or click one of the sidebar presets, then tweak it by hand).
  3. Capture cards re-sorting live with updated percentage match badges and clicking play on the top result.

### 2. FX Rack & `Tempo only` vs `Pitch + tempo` (`fx-rack-tempo-only.gif`)
- **TODO:** Capture `docs/media/fx-rack-tempo-only.gif` (plus optional `docs/media/fx-rack.png`)
- **What to show:**
  1. Select a sound card and open the right **FX Rack** panel.
  2. Move the **Speed / Pitch** slider (e.g. to `0.70x` or `1.45x`) in `Pitch + tempo` mode, watching the card waveform and duration update.
  3. Switch the **Speed mode** segmented toggle to **Tempo only**, then press **Preview with FX** so both modes and the UI control are clearly visible.

### 3. Native Drag-and-Drop 24-bit WAV into an NLE / DAW (`drag-to-nle.gif`)
- **TODO:** Capture `docs/media/drag-to-nle.gif`
- **What to show:**
  1. Position **SFX de Babel** side-by-side with Premiere Pro, DaVinci Resolve, CapCut, or FL Studio.
  2. Apply an FX preset (or keep `Clean`) and drag a sound card directly from the results list onto an audio track on the timeline.
  3. Show the drop succeeding and (optionally) a quick inset/crop inspecting the exported WAV's `LIST`/`INFO` tags (`INAM`, `IART`, `ICMT`, `ICOP`, `ISRC`).

### 4. License Badges, Warnings & Commercial-Only Filter (`license-badges.png`)
- **TODO:** Capture `docs/media/license-badges.png`
- **What to show:**
  1. A search result list containing a mix of licenses (e.g., `Creative Commons 0`, `CC BY 4.0` with `⚠ Attribution required`, `CC BY-NC` with `⛔ No commercial use`, `Mixkit Sound Effects Free License`, or `License unknown`).
  2. Optionally show the **Settings** toggle `Only sounds cleared for commercial use without attribution` enabled/disabled.

### 5. Local Library Indexing & Envelope Search (`local-library-indexing.png`)
- **TODO:** Capture `docs/media/local-library-indexing.png`
- **What to show:**
  1. Open **Settings** showing a selected folder path, the active **progress bar** (`Indexing X / Y`) + **Cancel** button, the **Envelope match: shape vs. level** slider (`65%`), and **Sound source** set to **Local library only**.
  2. Optionally include a second shot (`docs/media/local-library-results.png`) showing `Local` cards with the `Local file` badge and `LIBRARY_RANGE_RELAXED` banner when triggered.
