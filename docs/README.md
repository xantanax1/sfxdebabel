# Media & Screenshot Capture Guide

This folder is reserved for real UI screenshots and short workflow GIFs/WebMs referenced from the main README. No placeholder graphics or mock renders are checked in — capture each item from a live build of **SFX de Babel** and replace the `TODO` lines below with Markdown image/video links as you go.

## Capture Setup

- **Window size:** Default `1220 × 760` centered (or `1280 × 800` for NLE side-by-side drag-and-drop).
- **Desktop backdrop:** Plain dark wallpaper behind the frameless window so the transparent glassmorphism and hairline borders read cleanly.
- **Video / GIF:** Record at `30 fps`, keep GIFs under `10 MB` (e.g., `ffmpeg` palettegen or ScreenToGif).
- **Suggested asset folder:** Save final files under `docs/media/` (e.g., `docs/media/envelope-drawing.gif`).

## Captured Screenshots

- [Main Window — Envelope Scope, Results & License Badges](./media/main-window.jpg)
- [FX Rack — `Tempo only` (WSOLA) & DSP Chain](./media/fx-rack-tempo-only.png)
- [Settings — Local Library, Cosine/RMSE Weight & Commercial Filter](./media/settings-local-library.png)

## Planned Captures

### 1. Drawing an Envelope Contour & Live Re-ranking (`envelope-drawing.gif`)
- **TODO:** Capture `docs/media/envelope-drawing.gif` (static view captured in `main-window.jpg`)
- **What to show:**
  1. Start with a search query (e.g. `whoosh` or `impact`) and results loaded.
  2. Draw a custom curve on the **Volume Envelope** canvas (or click one of the sidebar presets, then tweak it by hand).
  3. Capture cards re-sorting live with updated percentage match badges and clicking play on the top result.

### 2. FX Rack & `Tempo only` vs `Pitch + tempo` (`fx-rack-tempo-only.gif`)
- **Captured:** [`docs/media/fx-rack-tempo-only.png`](./media/fx-rack-tempo-only.png)
- **TODO:** Capture animated `docs/media/fx-rack-tempo-only.gif`
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
- **Captured:** visible in [`docs/media/main-window.jpg`](./media/main-window.jpg) (`CC BY-NC 4.0`, `Attribution required`, `No commercial use`) and [`docs/media/settings-local-library.png`](./media/settings-local-library.png) (commercial-only toggle)

### 5. Local Library Indexing & Envelope Search (`local-library-indexing.png`)
- **Captured:** [`docs/media/settings-local-library.png`](./media/settings-local-library.png)
- **TODO:** Capture `docs/media/local-library-indexing.gif` showing the live progress bar + `Cancel` button during indexing, plus `Local` results.
- **What to show:**
  1. Open **Settings** showing a selected folder path, the active **progress bar** (`Indexing X / Y`) + **Cancel** button, the **Envelope match: shape vs. level** slider (`65%`), and **Sound source** set to **Local library only**.
  2. Optionally include a second shot (`docs/media/local-library-results.png`) showing `Local` cards with the `Local file` badge and `LIBRARY_RANGE_RELAXED` banner when triggered.
