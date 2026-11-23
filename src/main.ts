import { Channel, invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  applyDomTranslations,
  normalizeLocale,
  t,
  type Locale,
} from "./i18n";

const ENVELOPE_POINTS = 64;

type ProviderMode = "auto" | "freesound" | "mixkit";

interface AppSettings {
  freesoundApiKey: string;
  providerMode: ProviderMode;
  language: Locale;
}

interface CacheStats {
  entryCount: number;
  tempWavCount: number;
  dbSizeBytes: number;
}

interface SfxItem {
  id: string;
  provider: string;
  title: string;
  author: string;
  license: string;
  duration: number;
  previewUrl: string;
  sourceUrl: string;
  tags: string[];
  envelope: number[];
  matchScore: number | null;
}

interface SearchResponse {
  items: SfxItem[];
  providerUsed: string;
  warning: string | null;
  nextCursor: string | null;
}

interface PlaybackStatus {
  isPlaying: boolean;
  soundId: string | null;
  positionSecs: number;
  durationSecs: number;
}

interface FxParams {
  speedPitch: number;
  reverse: boolean;
  reverbMix: number;
  reverbRoomSize: number;
  reverbDamping: number;
  lowpassHz: number;
  highpassHz: number;
  drive: number;
  fadeInMs: number;
  fadeOutMs: number;
  normalize: boolean;
}

interface FxWaveformResult {
  envelope: number[];
  durationSecs: number;
}

function defaultFxParams(): FxParams {
  return {
    speedPitch: 1.0,
    reverse: false,
    reverbMix: 0.0,
    reverbRoomSize: 0.65,
    reverbDamping: 0.45,
    lowpassHz: 20000,
    highpassHz: 20,
    drive: 0.0,
    fadeInMs: 0,
    fadeOutMs: 0,
    normalize: true,
  };
}

// Application state
let userEnvelope: number[] | null = null;
let activePreset: string | null = null;
let isDrawing = false;
let lastDrawBin = -1;
let lastDrawVal = 0;

let currentItems: SfxItem[] = [];
let currentProvider = "Mixkit";
let currentLocale: Locale = "ru";
let savedLocale: Locale = "ru";
let lastSummaryMode: "ready" | "searching" | "results" | "error" = "ready";
let lastCacheEntryCount = 0;
let lastCacheStats: CacheStats | null = null;
let activeSearchQuery = "whoosh";
let nextCursor: string | null = null;
let isLoadingMore = false;
let selectedSoundId: string | null = null;
let fxParams: FxParams = defaultFxParams();
let fxDebounceTimer: number | null = null;
let fxWaveReqToken = 0;

let playback: PlaybackStatus = {
  isPlaying: false,
  soundId: null,
  positionSecs: 0,
  durationSecs: 0,
};
let playbackSnapshotTimeMs = 0;
let rafId: number | null = null;
let lastSearchToken = 0;

const seenIds = new Set<string>();
const loadedProviders = new Set<string>();
const drySnapshots = new Map<string, { envelope: number[]; duration: number }>();
const cardWaveCanvases = new Map<string, HTMLCanvasElement>();
const preparedWavPaths = new Map<string, string>();
const preparingWavPromises = new Map<string, Promise<string>>();

const scrollSentinel = document.createElement("div");
scrollSentinel.className = "load-more-row";

// Window & DOM references
const appWindow = getCurrentWindow();
type WindowResizeDirection = Parameters<typeof appWindow.startResizeDragging>[0];

const resizeHandles = Array.from(document.querySelectorAll<HTMLElement>(".resize-handle[data-resize]"));
const navItems = Array.from(document.querySelectorAll<HTMLButtonElement>(".nav-item[data-query]"));
const presetButtons = Array.from(
  document.querySelectorAll<HTMLButtonElement>(".preset-btn[data-preset]")
);

const searchForm = document.getElementById("search-form") as HTMLFormElement;
const searchInput = document.getElementById("search-input") as HTMLInputElement;
const searchBtn = document.getElementById("btn-search") as HTMLButtonElement;
const minDurInput = document.getElementById("min-duration") as HTMLInputElement;
const maxDurInput = document.getElementById("max-duration") as HTMLInputElement;
const durFill = document.getElementById("dual-range-fill") as HTMLDivElement;
const durReadout = document.getElementById("duration-readout") as HTMLSpanElement;
const volumeSlider = document.getElementById("volume-slider") as HTMLInputElement;
const volumeReadout = document.getElementById("volume-readout") as HTMLSpanElement;

const sidebarProvider = document.getElementById("sidebar-provider") as HTMLSpanElement;
const sidebarCacheCount = document.getElementById("sidebar-cache-count") as HTMLSpanElement;

const envelopeCanvas = document.getElementById("envelope-canvas") as HTMLCanvasElement;
const envelopeHint = document.getElementById("envelope-hint") as HTMLDivElement;
const envelopeStateLabel = document.getElementById("envelope-state-label") as HTMLSpanElement;
const resetEnvelopeBtn = document.getElementById("btn-reset-envelope") as HTMLButtonElement;
const resetEnvelopeSidebarBtn = document.getElementById(
  "btn-reset-envelope-sidebar"
) as HTMLButtonElement;

const resultsSummary = document.getElementById("results-summary") as HTMLDivElement;
const warningBanner = document.getElementById("warning-banner") as HTMLDivElement;
const resultsList = document.getElementById("results-list") as HTMLDivElement;

// FX Rack DOM references
const toggleFxBtn = document.getElementById("btn-toggle-fx") as HTMLButtonElement;
const fxActiveDot = document.getElementById("fx-active-dot") as HTMLSpanElement;
const fxRack = document.getElementById("fx-rack") as HTMLElement;
const fxActiveSound = document.getElementById("fx-active-sound") as HTMLSpanElement;
const fxResetBtn = document.getElementById("btn-fx-reset") as HTMLButtonElement;
const fxPreviewBtn = document.getElementById("btn-fx-preview") as HTMLButtonElement;
const fxPresetButtons = Array.from(
  document.querySelectorAll<HTMLButtonElement>(".fx-preset-btn[data-fx-preset]")
);

const fxSpeedInput = document.getElementById("fx-speed") as HTMLInputElement;
const fxReverseInput = document.getElementById("fx-reverse") as HTMLInputElement;
const fxReverbMixInput = document.getElementById("fx-reverb-mix") as HTMLInputElement;
const fxRoomSizeInput = document.getElementById("fx-room-size") as HTMLInputElement;
const fxDampingInput = document.getElementById("fx-damping") as HTMLInputElement;
const fxLowpassInput = document.getElementById("fx-lowpass") as HTMLInputElement;
const fxHighpassInput = document.getElementById("fx-highpass") as HTMLInputElement;
const fxDriveInput = document.getElementById("fx-drive") as HTMLInputElement;
const fxFadeInInput = document.getElementById("fx-fade-in") as HTMLInputElement;
const fxFadeOutInput = document.getElementById("fx-fade-out") as HTMLInputElement;
const fxNormalizeInput = document.getElementById("fx-normalize") as HTMLInputElement;

const valFxSpeed = document.getElementById("val-fx-speed") as HTMLSpanElement;
const valFxReverbMix = document.getElementById("val-fx-reverb-mix") as HTMLSpanElement;
const valFxRoomSize = document.getElementById("val-fx-room-size") as HTMLSpanElement;
const valFxDamping = document.getElementById("val-fx-damping") as HTMLSpanElement;
const valFxLowpass = document.getElementById("val-fx-lowpass") as HTMLSpanElement;
const valFxHighpass = document.getElementById("val-fx-highpass") as HTMLSpanElement;
const valFxDrive = document.getElementById("val-fx-drive") as HTMLSpanElement;
const valFxFadeIn = document.getElementById("val-fx-fade-in") as HTMLSpanElement;
const valFxFadeOut = document.getElementById("val-fx-fade-out") as HTMLSpanElement;

const fxSliders = [
  fxSpeedInput,
  fxReverbMixInput,
  fxRoomSizeInput,
  fxDampingInput,
  fxLowpassInput,
  fxHighpassInput,
  fxDriveInput,
  fxFadeInInput,
  fxFadeOutInput,
];

// Settings modal references
const settingsModal = document.getElementById("settings-modal") as HTMLDivElement;
const openSettingsBtn = document.getElementById("btn-open-settings") as HTMLButtonElement;
const closeSettingsBtn = document.getElementById("btn-close-settings") as HTMLButtonElement;
const cancelSettingsBtn = document.getElementById("btn-cancel-settings") as HTMLButtonElement;
const saveSettingsBtn = document.getElementById("btn-save-settings") as HTMLButtonElement;
const languageSelect = document.getElementById("input-language") as HTMLSelectElement;
const providerModeSelect = document.getElementById("input-provider-mode") as HTMLSelectElement;
const apiKeyInput = document.getElementById("input-api-key") as HTMLInputElement;
const toggleKeyBtn = document.getElementById("btn-toggle-key") as HTMLButtonElement;
const cacheStatsText = document.getElementById("cache-stats-text") as HTMLSpanElement;
const clearCacheBtn = document.getElementById("btn-clear-cache") as HTMLButtonElement;

const minimizeBtn = document.getElementById("btn-minimize") as HTMLButtonElement;
const closeBtn = document.getElementById("btn-close") as HTMLButtonElement;
const toastContainer = document.getElementById("toast-container") as HTMLDivElement;

function showToast(message: string): void {
  const el = document.createElement("div");
  el.className = "toast";
  el.textContent = message;
  toastContainer.appendChild(el);
  setTimeout(() => {
    el.remove();
  }, 3000);
}

function formatDuration(secs: number): string {
  const unit = t(currentLocale, "unit.sec");
  if (!Number.isFinite(secs) || secs < 0) return `0.00 ${unit}`;
  return `${secs.toFixed(2)} ${unit}`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} ${t(currentLocale, "unit.b")}`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} ${t(currentLocale, "unit.kb")}`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} ${t(currentLocale, "unit.mb")}`;
}

function sliderToHz(sliderVal: number, minHz: number, maxHz: number): number {
  const t = Math.max(0, Math.min(1, sliderVal / 1000));
  return minHz * Math.pow(maxHz / minHz, t);
}

function hzToSlider(hz: number, minHz: number, maxHz: number): number {
  const clamped = Math.max(minHz, Math.min(maxHz, hz));
  return Math.round((Math.log(clamped / minHz) / Math.log(maxHz / minHz)) * 1000);
}

function formatHz(hz: number): string {
  if (hz >= 1000) {
    return `${(hz / 1000).toFixed(1)} kHz`;
  }
  return `${Math.round(hz)} Hz`;
}

function isFxClean(p: FxParams): boolean {
  return (
    Math.abs(p.speedPitch - 1.0) < 0.005 &&
    !p.reverse &&
    p.reverbMix < 0.005 &&
    p.lowpassHz >= 19950 &&
    p.highpassHz <= 21 &&
    p.drive < 0.005 &&
    p.fadeInMs === 0 &&
    p.fadeOutMs === 0 &&
    p.normalize
  );
}

function updateRangeFill(input: HTMLInputElement): void {
  const min = parseFloat(input.min || "0");
  const max = parseFloat(input.max || "100");
  const val = parseFloat(input.value || "0");
  const pct = max > min ? ((val - min) / (max - min)) * 100 : 0;
  input.style.setProperty("--pct", `${Math.max(0, Math.min(100, pct)).toFixed(1)}%`);
}

function syncFxControlsFromParams(): void {
  fxSpeedInput.value = fxParams.speedPitch.toFixed(2);
  fxReverseInput.checked = fxParams.reverse;
  fxReverbMixInput.value = String(Math.round(fxParams.reverbMix * 100));
  fxRoomSizeInput.value = String(Math.round(fxParams.reverbRoomSize * 100));
  fxDampingInput.value = String(Math.round(fxParams.reverbDamping * 100));
  fxLowpassInput.value = String(hzToSlider(fxParams.lowpassHz, 200, 20000));
  fxHighpassInput.value = String(hzToSlider(fxParams.highpassHz, 20, 5000));
  fxDriveInput.value = String(Math.round(fxParams.drive * 100));
  fxFadeInInput.value = String(fxParams.fadeInMs);
  fxFadeOutInput.value = String(fxParams.fadeOutMs);
  fxNormalizeInput.checked = fxParams.normalize;

  valFxSpeed.textContent = `${fxParams.speedPitch.toFixed(2)}x`;
  valFxReverbMix.textContent = `${Math.round(fxParams.reverbMix * 100)}%`;
  valFxRoomSize.textContent = `${Math.round(fxParams.reverbRoomSize * 100)}%`;
  valFxDamping.textContent = `${Math.round(fxParams.reverbDamping * 100)}%`;
  valFxLowpass.textContent = formatHz(fxParams.lowpassHz);
  valFxHighpass.textContent = formatHz(fxParams.highpassHz);
  valFxDrive.textContent = `${Math.round(fxParams.drive * 100)}%`;
  valFxFadeIn.textContent = `${fxParams.fadeInMs} ms`;
  valFxFadeOut.textContent = `${fxParams.fadeOutMs} ms`;

  fxSliders.forEach(updateRangeFill);
  fxActiveDot.classList.toggle("hidden", isFxClean(fxParams));
}

function readFxParamsFromControls(): void {
  fxParams = {
    speedPitch: parseFloat(fxSpeedInput.value),
    reverse: fxReverseInput.checked,
    reverbMix: parseFloat(fxReverbMixInput.value) / 100,
    reverbRoomSize: parseFloat(fxRoomSizeInput.value) / 100,
    reverbDamping: parseFloat(fxDampingInput.value) / 100,
    lowpassHz: Math.round(sliderToHz(parseFloat(fxLowpassInput.value), 200, 20000)),
    highpassHz: Math.round(sliderToHz(parseFloat(fxHighpassInput.value), 20, 5000)),
    drive: parseFloat(fxDriveInput.value) / 100,
    fadeInMs: parseInt(fxFadeInInput.value, 10) || 0,
    fadeOutMs: parseInt(fxFadeOutInput.value, 10) || 0,
    normalize: fxNormalizeInput.checked,
  };

  valFxSpeed.textContent = `${fxParams.speedPitch.toFixed(2)}x`;
  valFxReverbMix.textContent = `${Math.round(fxParams.reverbMix * 100)}%`;
  valFxRoomSize.textContent = `${Math.round(fxParams.reverbRoomSize * 100)}%`;
  valFxDamping.textContent = `${Math.round(fxParams.reverbDamping * 100)}%`;
  valFxLowpass.textContent = formatHz(fxParams.lowpassHz);
  valFxHighpass.textContent = formatHz(fxParams.highpassHz);
  valFxDrive.textContent = `${Math.round(fxParams.drive * 100)}%`;
  valFxFadeIn.textContent = `${fxParams.fadeInMs} ms`;
  valFxFadeOut.textContent = `${fxParams.fadeOutMs} ms`;

  fxSliders.forEach(updateRangeFill);
  fxActiveDot.classList.toggle("hidden", isFxClean(fxParams));
  fxPresetButtons.forEach((b) => b.classList.toggle("active", b.dataset.fxPreset === "clean" && isFxClean(fxParams)));
}

function normalizeEnvelope(curve: number[]): number[] {
  const out = curve.slice(0, ENVELOPE_POINTS).map((v) => Math.max(0, Math.min(1, v)));
  while (out.length < ENVELOPE_POINTS) out.push(0);
  const peak = Math.max(...out, 0);
  if (peak > 1e-6) {
    return out.map((v) => v / peak);
  }
  return out;
}

function computeSimilarity(target: number[], candidate: number[]): number {
  const a = normalizeEnvelope(target);
  const b = normalizeEnvelope(candidate);
  const energyA = a.reduce((acc, v) => acc + v * v, 0);
  const energyB = b.reduce((acc, v) => acc + v * v, 0);
  if (energyA < 1e-7 || energyB < 1e-7) return 0;

  let best = 0;
  for (let shift = -2; shift <= 2; shift++) {
    let dot = 0;
    let normA = 0;
    let normB = 0;
    let sqErr = 0;
    for (let i = 0; i < ENVELOPE_POINTS; i++) {
      const va = a[i];
      const j = i + shift;
      const vb = j >= 0 && j < ENVELOPE_POINTS ? b[j] : 0;
      dot += va * vb;
      normA += va * va;
      normB += vb * vb;
      const diff = va - vb;
      sqErr += diff * diff;
    }
    const denom = Math.sqrt(normA) * Math.sqrt(normB);
    const cos = denom > 1e-7 ? Math.max(0, Math.min(1, dot / denom)) : 0;
    const rmse = Math.max(0, Math.min(1, Math.sqrt(sqErr / ENVELOPE_POINTS)));
    const combined = Math.max(0, Math.min(1, 0.65 * cos + 0.35 * (1 - rmse)));
    if (combined > best) best = combined;
  }
  return Math.round(best * 1000) / 10;
}

function generatePresetCurve(name: string): number[] {
  const out = new Array<number>(ENVELOPE_POINTS).fill(0);
  for (let i = 0; i < ENVELOPE_POINTS; i++) {
    const t = i / (ENVELOPE_POINTS - 1);
    switch (name) {
      case "whoosh": {
        out[i] = Math.exp(-Math.pow((t - 0.5) / 0.21, 2));
        break;
      }
      case "impact": {
        out[i] = i < 2 ? (i === 0 ? 0.92 : 1.0) : Math.exp(-((t - 0.03) * 6.5));
        break;
      }
      case "sub-drop": {
        const attack = Math.min(1, t / 0.08);
        out[i] = attack * Math.exp(-Math.max(0, t - 0.08) * 2.2);
        break;
      }
      case "ui-click": {
        out[i] = i === 0 ? 1.0 : i === 1 ? 0.52 : Math.exp(-t * 24.0) * 0.35;
        break;
      }
      case "riser": {
        out[i] = t > 0.94 ? Math.max(0.15, (1.0 - t) / 0.06) : Math.pow(t / 0.94, 2.3);
        break;
      }
    }
  }
  return normalizeEnvelope(out);
}

function resizeCanvasDpr(canvas: HTMLCanvasElement): CanvasRenderingContext2D | null {
  const rect = canvas.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  const targetW = Math.max(1, Math.round(rect.width * dpr));
  const targetH = Math.max(1, Math.round(rect.height * dpr));
  if (canvas.width !== targetW || canvas.height !== targetH) {
    canvas.width = targetW;
    canvas.height = targetH;
  }
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  return ctx;
}

function drawSmoothCurvePath(
  ctx: CanvasRenderingContext2D,
  pts: Array<{ x: number; y: number }>
): void {
  ctx.beginPath();
  ctx.moveTo(pts[0].x, pts[0].y);
  for (let i = 0; i < pts.length - 1; i++) {
    const midX = (pts[i].x + pts[i + 1].x) * 0.5;
    const midY = (pts[i].y + pts[i + 1].y) * 0.5;
    ctx.quadraticCurveTo(pts[i].x, pts[i].y, midX, midY);
  }
  ctx.lineTo(pts[pts.length - 1].x, pts[pts.length - 1].y);
}

function renderEnvelopeCanvas(): void {
  const ctx = resizeCanvasDpr(envelopeCanvas);
  if (!ctx) return;

  const width = envelopeCanvas.clientWidth;
  const height = envelopeCanvas.clientHeight;
  ctx.clearRect(0, 0, width, height);

  const padLeft = 40;
  const padRight = 12;
  const padTop = 12;
  const padBottom = 20;
  const plotW = Math.max(20, width - padLeft - padRight);
  const plotH = Math.max(16, height - padTop - padBottom);

  ctx.strokeStyle = "rgba(237, 237, 237, 0.05)";
  ctx.lineWidth = 1;
  for (let gx = 0; gx <= 4; gx++) {
    const x = Math.round(padLeft + (plotW * gx) / 4) + 0.5;
    ctx.beginPath();
    ctx.moveTo(x, padTop);
    ctx.lineTo(x, padTop + plotH);
    ctx.stroke();
  }
  for (let gy = 0; gy <= 2; gy++) {
    const y = Math.round(padTop + (plotH * gy) / 2) + 0.5;
    ctx.beginPath();
    ctx.moveTo(padLeft, y);
    ctx.lineTo(padLeft + plotW, y);
    ctx.stroke();
  }

  ctx.fillStyle = "rgba(133, 133, 139, 0.78)";
  ctx.font = '10px "JetBrains Mono", "Cascadia Mono", Consolas, monospace';
  ctx.textAlign = "right";
  ctx.textBaseline = "middle";
  ctx.fillText("0 dB", padLeft - 8, padTop + 2);
  ctx.fillText("-12", padLeft - 8, padTop + plotH * 0.5);
  ctx.fillText("-∞", padLeft - 8, padTop + plotH - 2);

  ctx.textBaseline = "bottom";
  ctx.textAlign = "left";
  ctx.fillText("0%", padLeft, height - 4);
  ctx.textAlign = "center";
  ctx.fillText("50%", padLeft + plotW * 0.5, height - 4);
  ctx.textAlign = "right";
  ctx.fillText("100%", padLeft + plotW, height - 4);

  const refId = playback.soundId ?? selectedSoundId;
  const activeItem = refId ? currentItems.find((item) => item.id === refId) : undefined;
  if (activeItem && activeItem.envelope.length > 0) {
    const refPts = activeItem.envelope.map((v, idx) => ({
      x: padLeft + (idx / (activeItem.envelope.length - 1)) * plotW,
      y: padTop + (1 - Math.max(0, Math.min(1, v))) * plotH,
    }));
    ctx.save();
    ctx.setLineDash([3, 3]);
    drawSmoothCurvePath(ctx, refPts);
    ctx.strokeStyle = "rgba(161, 161, 166, 0.36)";
    ctx.lineWidth = 1.25;
    ctx.stroke();
    ctx.restore();
  }

  if (!userEnvelope) {
    envelopeHint.classList.remove("hidden");
    resetEnvelopeBtn.classList.add("hidden");
    envelopeStateLabel.textContent = activeItem
      ? t(currentLocale, "scope.ref", { name: activeItem.title })
      : t(currentLocale, "scope.notSet");
    return;
  }

  envelopeHint.classList.add("hidden");
  resetEnvelopeBtn.classList.remove("hidden");
  envelopeStateLabel.textContent = activePreset
    ? t(currentLocale, "scope.preset", { name: activePreset })
    : t(currentLocale, "scope.points64");

  const pts = userEnvelope.map((v, idx) => ({
    x: padLeft + (idx / (ENVELOPE_POINTS - 1)) * plotW,
    y: padTop + (1 - Math.max(0, Math.min(1, v))) * plotH,
  }));

  const baseY = padTop + plotH;
  const grad = ctx.createLinearGradient(0, padTop, 0, baseY);
  grad.addColorStop(0, "rgba(237, 237, 237, 0.14)");
  grad.addColorStop(1, "rgba(237, 237, 237, 0.01)");

  ctx.beginPath();
  ctx.moveTo(pts[0].x, baseY);
  ctx.lineTo(pts[0].x, pts[0].y);
  for (let i = 0; i < pts.length - 1; i++) {
    const midX = (pts[i].x + pts[i + 1].x) * 0.5;
    const midY = (pts[i].y + pts[i + 1].y) * 0.5;
    ctx.quadraticCurveTo(pts[i].x, pts[i].y, midX, midY);
  }
  ctx.lineTo(pts[pts.length - 1].x, pts[pts.length - 1].y);
  ctx.lineTo(pts[pts.length - 1].x, baseY);
  ctx.closePath();
  ctx.fillStyle = grad;
  ctx.fill();

  drawSmoothCurvePath(ctx, pts);
  ctx.strokeStyle = "#ededed";
  ctx.lineWidth = 1.75;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.stroke();
}

function updateLiveMatchScores(): void {
  if (currentItems.length === 0) return;

  if (userEnvelope) {
    const norm = normalizeEnvelope(userEnvelope);
    for (const item of currentItems) {
      const dry = drySnapshots.get(item.id);
      const envToScore = dry ? dry.envelope : item.envelope;
      item.matchScore = computeSimilarity(norm, envToScore);
    }
    currentItems.sort((a, b) => (b.matchScore ?? 0) - (a.matchScore ?? 0));
  } else {
    for (const item of currentItems) {
      item.matchScore = null;
    }
  }

  renderResultsList();
}

function pointerEventToBinAndVal(ev: PointerEvent): { bin: number; val: number } {
  const rect = envelopeCanvas.getBoundingClientRect();
  const padLeft = 40;
  const padRight = 12;
  const padTop = 12;
  const padBottom = 20;
  const plotW = Math.max(20, rect.width - padLeft - padRight);
  const plotH = Math.max(16, rect.height - padTop - padBottom);

  const nx = Math.max(0, Math.min(1, (ev.clientX - rect.left - padLeft) / plotW));
  const ny = Math.max(0, Math.min(1, (ev.clientY - rect.top - padTop) / plotH));
  const bin = Math.round(nx * (ENVELOPE_POINTS - 1));
  const val = Math.max(0.02, Math.min(1, 1 - ny));
  return { bin, val };
}

function applyEnvelopeStroke(bin: number, val: number): void {
  if (!userEnvelope) {
    userEnvelope = new Array<number>(ENVELOPE_POINTS).fill(0.03);
  }
  activePreset = null;
  presetButtons.forEach((b) => b.classList.remove("active"));

  if (lastDrawBin < 0) {
    userEnvelope[bin] = val;
  } else {
    const step = bin >= lastDrawBin ? 1 : -1;
    const dist = Math.abs(bin - lastDrawBin);
    for (let i = 0; i <= dist; i++) {
      const idx = lastDrawBin + i * step;
      const frac = dist === 0 ? 1 : i / dist;
      userEnvelope[idx] = lastDrawVal + (val - lastDrawVal) * frac;
    }
  }
  lastDrawBin = bin;
  lastDrawVal = val;
  renderEnvelopeCanvas();
}

envelopeCanvas.addEventListener("pointerdown", (ev) => {
  if (ev.button !== 0) return;
  isDrawing = true;
  lastDrawBin = -1;
  envelopeCanvas.setPointerCapture(ev.pointerId);
  const { bin, val } = pointerEventToBinAndVal(ev);
  applyEnvelopeStroke(bin, val);
});

envelopeCanvas.addEventListener("pointermove", (ev) => {
  if (!isDrawing) return;
  const { bin, val } = pointerEventToBinAndVal(ev);
  applyEnvelopeStroke(bin, val);
});

function finishDrawing(ev: PointerEvent): void {
  if (!isDrawing) return;
  isDrawing = false;
  lastDrawBin = -1;
  if (envelopeCanvas.hasPointerCapture(ev.pointerId)) {
    envelopeCanvas.releasePointerCapture(ev.pointerId);
  }
  if (userEnvelope) {
    userEnvelope = normalizeEnvelope(userEnvelope);
    renderEnvelopeCanvas();
    updateLiveMatchScores();
  }
}

envelopeCanvas.addEventListener("pointerup", finishDrawing);
envelopeCanvas.addEventListener("pointercancel", finishDrawing);

function resetUserEnvelope(): void {
  userEnvelope = null;
  activePreset = null;
  presetButtons.forEach((b) => b.classList.remove("active"));
  renderEnvelopeCanvas();
  updateLiveMatchScores();
}

presetButtons.forEach((btn) => {
  btn.addEventListener("click", () => {
    const preset = btn.dataset.preset;
    if (!preset) return;
    activePreset = btn.textContent?.trim() ?? preset;
    presetButtons.forEach((b) => b.classList.toggle("active", b === btn));
    userEnvelope = generatePresetCurve(preset);
    renderEnvelopeCanvas();
    updateLiveMatchScores();
  });
});

resetEnvelopeBtn.addEventListener("click", resetUserEnvelope);
resetEnvelopeSidebarBtn.addEventListener("click", resetUserEnvelope);

// Sidebar category navigation
function syncCategoryHighlight(query: string): void {
  const norm = query.trim().toLowerCase();
  navItems.forEach((btn) => {
    const q = (btn.dataset.query ?? "").trim().toLowerCase();
    btn.classList.toggle("active", q === norm);
  });
}

navItems.forEach((btn) => {
  btn.addEventListener("click", () => {
    const q = btn.dataset.query ?? "";
    searchInput.value = q;
    syncCategoryHighlight(q);
    void runSearch(q);
  });
});

searchInput.addEventListener("input", () => {
  syncCategoryHighlight(searchInput.value);
});

// Dual duration slider synchronization
function syncDurationSlider(changed: "min" | "max"): void {
  let minVal = parseFloat(minDurInput.value);
  let maxVal = parseFloat(maxDurInput.value);
  const minGap = 0.2;

  if (changed === "min" && minVal > maxVal - minGap) {
    minVal = Math.max(0.1, Number((maxVal - minGap).toFixed(1)));
    minDurInput.value = minVal.toFixed(1);
  } else if (changed === "max" && maxVal < minVal + minGap) {
    maxVal = Math.min(10.0, Number((minVal + minGap).toFixed(1)));
    maxDurInput.value = maxVal.toFixed(1);
  }

  const leftPct = ((minVal - 0.1) / 9.9) * 100;
  const rightPct = ((maxVal - 0.1) / 9.9) * 100;
  durFill.style.left = `${leftPct}%`;
  durFill.style.width = `${Math.max(0, rightPct - leftPct)}%`;
  durReadout.textContent = `${minVal.toFixed(1)}–${maxVal.toFixed(1)} ${t(currentLocale, "unit.sec")}`;
}

minDurInput.addEventListener("input", () => syncDurationSlider("min"));
maxDurInput.addEventListener("input", () => syncDurationSlider("max"));

volumeSlider.addEventListener("input", () => {
  const pct = Number(volumeSlider.value);
  volumeReadout.textContent = `${pct}%`;
  void invoke("set_volume", { volume: pct / 100 }).catch(() => {});
});

// Mini waveform rendering per card
function drawCardWaveform(
  canvas: HTMLCanvasElement,
  envelope: number[],
  progressRatio: number,
  isCurrent: boolean
): void {
  const ctx = resizeCanvasDpr(canvas);
  if (!ctx) return;

  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  ctx.clearRect(0, 0, w, h);

  const count = envelope.length || ENVELOPE_POINTS;
  const step = w / count;
  const barW = Math.max(1.5, step * 0.56);
  const midY = h * 0.5;

  for (let i = 0; i < count; i++) {
    const v = Math.max(0.06, Math.min(1, envelope[i] ?? 0));
    const barH = Math.max(2, v * (h - 4));
    const x = i * step + (step - barW) * 0.5;
    const y = midY - barH * 0.5;
    const binRatio = (i + 0.5) / count;

    if (isCurrent && binRatio <= progressRatio) {
      ctx.fillStyle = "#ededed";
    } else if (isCurrent) {
      ctx.fillStyle = "rgba(237, 237, 237, 0.30)";
    } else {
      ctx.fillStyle = "rgba(237, 237, 237, 0.42)";
    }
    ctx.fillRect(x, y, barW, barH);
  }

  if (isCurrent && progressRatio > 0 && progressRatio < 1) {
    const playX = Math.max(1, Math.min(w - 1, progressRatio * w));
    ctx.strokeStyle = "#ededed";
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(playX, 0);
    ctx.lineTo(playX, h);
    ctx.stroke();
  }
}

function updateCardVisualMeta(soundId: string, item: SfxItem): void {
  const card = resultsList.querySelector<HTMLElement>(`.sfx-card[data-sound-id="${soundId}"]`);
  if (card) {
    const durEl = card.querySelector<HTMLElement>(".sfx-duration");
    if (durEl) {
      durEl.textContent = formatDuration(item.duration);
    }
  }
  const canvas = cardWaveCanvases.get(soundId);
  if (canvas) {
    const isCurrent = playback.soundId === soundId;
    const pos = getInterpolatedPlaybackPos();
    const ratio = isCurrent && playback.durationSecs > 0 ? pos / playback.durationSecs : 0;
    drawCardWaveform(canvas, item.envelope, ratio, isCurrent);
  }
}

function selectSoundForFx(soundId: string): void {
  if (selectedSoundId && selectedSoundId !== soundId) {
    const prevDry = drySnapshots.get(selectedSoundId);
    const prevItem = currentItems.find((x) => x.id === selectedSoundId);
    if (prevDry && prevItem) {
      prevItem.envelope = [...prevDry.envelope];
      prevItem.duration = prevDry.duration;
      updateCardVisualMeta(selectedSoundId, prevItem);
    }
  }

  selectedSoundId = soundId;
  const item = currentItems.find((x) => x.id === soundId);
  fxActiveSound.textContent = item ? item.title : t(currentLocale, "fx.selectSound");

  resultsList.querySelectorAll<HTMLElement>(".sfx-card").forEach((el) => {
    el.classList.toggle("is-selected", el.dataset.soundId === soundId);
  });

  if (item && !isFxClean(fxParams)) {
    void updateFxWaveformNow();
  } else {
    renderEnvelopeCanvas();
  }
}

async function updateFxWaveformNow(): Promise<void> {
  if (!selectedSoundId) return;
  const targetId = selectedSoundId;
  const item = currentItems.find((x) => x.id === targetId);
  if (!item) return;

  if (isFxClean(fxParams)) {
    const dry = drySnapshots.get(targetId);
    if (dry) {
      item.envelope = [...dry.envelope];
      item.duration = dry.duration;
      updateCardVisualMeta(targetId, item);
      renderEnvelopeCanvas();
    }
    return;
  }

  const token = ++fxWaveReqToken;
  try {
    const res = await invoke<FxWaveformResult>("render_fx_waveform", {
      soundId: targetId,
      fx: fxParams,
    });
    if (token !== fxWaveReqToken || selectedSoundId !== targetId) return;

    item.envelope = res.envelope;
    item.duration = res.durationSecs;
    updateCardVisualMeta(targetId, item);
    renderEnvelopeCanvas();
  } catch {
    // Non-fatal during rapid scrubbing
  }
}

function scheduleFxWaveformUpdate(): void {
  if (fxDebounceTimer !== null) {
    window.clearTimeout(fxDebounceTimer);
  }
  fxDebounceTimer = window.setTimeout(() => {
    fxDebounceTimer = null;
    void updateFxWaveformNow();
  }, 80);
}

function applyFxPreset(name: string): void {
  const next = defaultFxParams();
  switch (name) {
    case "clean":
      break;
    case "slowed-reverb":
      next.speedPitch = 0.8;
      next.reverbMix = 0.42;
      next.reverbRoomSize = 0.78;
      next.reverbDamping = 0.48;
      next.lowpassHz = 14000;
      break;
    case "speedup":
      next.speedPitch = 1.32;
      next.highpassHz = 60;
      next.drive = 0.12;
      break;
    case "reverse-riser":
      next.reverse = true;
      next.reverbMix = 0.28;
      next.reverbRoomSize = 0.72;
      next.fadeInMs = 40;
      next.fadeOutMs = 60;
      break;
    case "muffled":
      next.lowpassHz = 850;
      next.highpassHz = 80;
      next.reverbMix = 0.35;
      next.reverbRoomSize = 0.6;
      next.reverbDamping = 0.72;
      break;
    case "heavy-impact":
      next.speedPitch = 0.9;
      next.drive = 0.55;
      next.lowpassHz = 11000;
      next.highpassHz = 35;
      next.reverbMix = 0.16;
      next.reverbRoomSize = 0.5;
      break;
  }

  fxParams = next;
  syncFxControlsFromParams();
  fxPresetButtons.forEach((b) => b.classList.toggle("active", b.dataset.fxPreset === name));

  if (selectedSoundId) {
    const item = currentItems.find((x) => x.id === selectedSoundId);
    if (item) {
      void togglePlay(item, 0);
      return;
    }
  }
  scheduleFxWaveformUpdate();
}

function buildDragIconDataUrl(title: string): string {
  const canvas = document.createElement("canvas");
  canvas.width = 236;
  canvas.height = 44;
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    return "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
  }

  ctx.fillStyle = "rgba(20, 20, 22, 0.96)";
  ctx.strokeStyle = "rgba(237, 237, 237, 0.24)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.roundRect(1, 1, 234, 42, 8);
  ctx.fill();
  ctx.stroke();

  ctx.fillStyle = "#ededed";
  ctx.font = '500 12px -apple-system, "Segoe UI", sans-serif';
  const tag = isFxClean(fxParams) ? "24-bit" : "FX 24-bit";
  const label = title.length > 18 ? `${title.slice(0, 17)}…` : title;
  ctx.fillText(`${tag} · ${label}`, 12, 26);

  return canvas.toDataURL("image/png");
}

async function getOrPrepareWav(item: SfxItem): Promise<string> {
  const cacheKey = `${item.id}:${JSON.stringify(fxParams)}`;
  const existing = preparedWavPaths.get(cacheKey);
  if (existing) return existing;

  const inflight = preparingWavPromises.get(cacheKey);
  if (inflight) return inflight;

  const promise = invoke<string>("prepare_drag_file", {
    soundId: item.id,
    fx: fxParams,
  })
    .then((wavPath) => {
      preparedWavPaths.set(cacheKey, wavPath);
      preparingWavPromises.delete(cacheKey);
      return wavPath;
    })
    .catch((err) => {
      preparingWavPromises.delete(cacheKey);
      throw err;
    });

  preparingWavPromises.set(cacheKey, promise);
  return promise;
}

async function togglePlay(item: SfxItem, startSecs?: number): Promise<void> {
  if (selectedSoundId !== item.id) {
    selectSoundForFx(item.id);
  }

  if (playback.isPlaying && playback.soundId === item.id && startSecs === undefined) {
    try {
      await invoke("stop_sfx");
      playback = { ...playback, isPlaying: false };
      updatePlaybackUi();
      renderEnvelopeCanvas();
    } catch (err) {
      showToast(t(currentLocale, "toast.stopErr", { err: String(err) }));
    }
    return;
  }

  try {
    const updatedEnv = await invoke<number[]>("preview_sfx_with_fx", {
      soundId: item.id,
      fx: fxParams,
      startSecs: startSecs ?? null,
      volume: Number(volumeSlider.value) / 100,
    });
    const status = await invoke<PlaybackStatus>("get_playback_status");

    item.envelope = updatedEnv;
    if (status.durationSecs > 0) {
      item.duration = status.durationSecs;
    }

    playback = status;
    playbackSnapshotTimeMs = performance.now();

    updateCardVisualMeta(item.id, item);
    renderEnvelopeCanvas();
    startPlaybackLoop();
    updatePlaybackUi();
  } catch (err) {
    showToast(String(err));
  }
}

function getInterpolatedPlaybackPos(): number {
  if (!playback.isPlaying) return playback.positionSecs;
  const elapsed = (performance.now() - playbackSnapshotTimeMs) / 1000;
  return Math.min(playback.durationSecs, playback.positionSecs + elapsed);
}

function updatePlaybackUi(): void {
  const cards = resultsList.querySelectorAll<HTMLElement>(".sfx-card");
  const pos = getInterpolatedPlaybackPos();

  cards.forEach((card) => {
    const id = card.dataset.soundId ?? "";
    const isCurrent = playback.soundId === id;
    const isActive = isCurrent && playback.isPlaying;
    card.classList.toggle("is-playing", isActive);
    card.classList.toggle("is-selected", selectedSoundId === id);

    const liveDot = card.querySelector<HTMLElement>(".live-dot");
    if (liveDot) {
      liveDot.classList.toggle("hidden", !isActive);
    }

    const playBtn = card.querySelector<HTMLButtonElement>(".play-btn");
    if (playBtn) {
      playBtn.innerHTML = isActive
        ? `<svg viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="1.5"/></svg>`
        : `<svg viewBox="0 0 24 24" fill="currentColor"><polygon points="8,5 19,12 8,19"/></svg>`;
    }

    const canvas = cardWaveCanvases.get(id);
    const item = currentItems.find((x) => x.id === id);
    if (canvas && item) {
      const ratio = isCurrent && playback.durationSecs > 0 ? pos / playback.durationSecs : 0;
      drawCardWaveform(canvas, item.envelope, ratio, isCurrent);
    }
  });
}

function startPlaybackLoop(): void {
  if (rafId !== null) return;
  const tick = () => {
    if (!playback.isPlaying) {
      rafId = null;
      return;
    }
    const pos = getInterpolatedPlaybackPos();
    if (playback.durationSecs > 0 && pos >= playback.durationSecs) {
      playback = {
        ...playback,
        isPlaying: false,
        positionSecs: 0,
      };
      updatePlaybackUi();
      rafId = null;
      return;
    }
    updatePlaybackUi();
    rafId = requestAnimationFrame(tick);
  };
  rafId = requestAnimationFrame(tick);
}

function attachCardDragBehavior(card: HTMLElement, item: SfxItem): void {
  card.setAttribute("draggable", "false");

  card.addEventListener("pointerdown", (ev) => {
    if (ev.button !== 0) return;
    const target = ev.target as HTMLElement;
    if (target.closest("button") || target.closest(".wave-wrap")) {
      return;
    }

    selectSoundForFx(item.id);

    const startX = ev.clientX;
    const startY = ev.clientY;
    let started = false;

    void getOrPrepareWav(item).catch(() => {});

    const onMove = async (moveEv: PointerEvent) => {
      if (started) return;
      const dist = Math.hypot(moveEv.clientX - startX, moveEv.clientY - startY);
      if (dist < 6) return;

      started = true;
      cleanup();

      try {
        const wavPath = await getOrPrepareWav(item);
        const onEvent = new Channel();
        onEvent.onmessage = (payload: unknown) => {
          const res = payload as { result?: string };
          if (res?.result === "Dropped") {
            showToast(t(currentLocale, "toast.exported", { title: item.title }));
          }
        };

        await invoke("plugin:drag|start_drag", {
          item: [wavPath],
          image: buildDragIconDataUrl(item.title),
          options: { mode: "copy" },
          onEvent,
        });
      } catch (err) {
        showToast(t(currentLocale, "toast.dragErr", { err: String(err) }));
      }
    };

    const cleanup = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", cleanup);
      window.removeEventListener("pointercancel", cleanup);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", cleanup);
    window.addEventListener("pointercancel", cleanup);
  });
}

function createSfxCardElement(item: SfxItem): HTMLElement {
  const card = document.createElement("article");
  card.className = "sfx-card";
  card.dataset.soundId = item.id;

  const playBtn = document.createElement("button");
  playBtn.type = "button";
  playBtn.className = "play-btn";
  playBtn.title = t(currentLocale, "card.playTitle");
  playBtn.innerHTML = `<svg viewBox="0 0 24 24" fill="currentColor"><polygon points="8,5 19,12 8,19"/></svg>`;
  playBtn.addEventListener("click", (e) => {
    e.stopPropagation();
    void togglePlay(item);
  });

  const info = document.createElement("div");
  info.className = "sfx-info";

  const titleRow = document.createElement("div");
  titleRow.className = "sfx-title-row";
  const liveDot = document.createElement("span");
  liveDot.className = "live-dot hidden";
  const titleEl = document.createElement("div");
  titleEl.className = "sfx-title";
  titleEl.textContent = item.title;
  titleEl.title = item.title;
  titleRow.append(liveDot, titleEl);

  const metaEl = document.createElement("div");
  metaEl.className = "sfx-meta";
  const durSpan = document.createElement("span");
  durSpan.className = "sfx-duration";
  durSpan.textContent = formatDuration(item.duration);
  const metaText = document.createElement("span");
  metaText.textContent = `${item.provider} · ${item.author}`;
  metaEl.append(durSpan, metaText);

  info.append(titleRow, metaEl);

  const waveWrap = document.createElement("div");
  waveWrap.className = "wave-wrap";
  waveWrap.title = t(currentLocale, "card.seekTitle");
  const waveCanvas = document.createElement("canvas");
  waveCanvas.className = "wave-canvas";
  waveWrap.appendChild(waveCanvas);
  cardWaveCanvases.set(item.id, waveCanvas);

  waveWrap.addEventListener("click", (e) => {
    e.stopPropagation();
    const rect = waveWrap.getBoundingClientRect();
    const ratio = Math.max(0, Math.min(1, (e.clientX - rect.left) / Math.max(1, rect.width)));
    void togglePlay(item, ratio * item.duration);
  });

  const actions = document.createElement("div");
  actions.className = "sfx-actions";

  if (item.matchScore !== null) {
    const badge = document.createElement("span");
    badge.className = `match-badge${item.matchScore >= 80 ? " high" : ""}`;
    badge.textContent = `${item.matchScore.toFixed(0)}%`;
    badge.title = t(currentLocale, "card.matchTitle");
    actions.appendChild(badge);
  }

  const wavBtn = document.createElement("button");
  wavBtn.type = "button";
  wavBtn.className = "wav-btn";
  wavBtn.textContent = "WAV";
  wavBtn.title = t(currentLocale, "card.wavTitle");
  wavBtn.addEventListener("click", async (e) => {
    e.stopPropagation();
    selectSoundForFx(item.id);
    try {
      const path = await getOrPrepareWav(item);
      showToast(t(currentLocale, "toast.wavSaved", { path }));
    } catch (err) {
      showToast(String(err));
    }
  });
  actions.appendChild(wavBtn);

  card.append(playBtn, info, waveWrap, actions);
  attachCardDragBehavior(card, item);
  return card;
}

function updateScrollSentinel(errorMsg?: string): void {
  scrollSentinel.innerHTML = "";
  if (currentItems.length === 0) {
    scrollSentinel.classList.add("hidden");
    return;
  }
  scrollSentinel.classList.remove("hidden");

  if (isLoadingMore) {
    const span = document.createElement("span");
    span.textContent = t(currentLocale, "results.loadingMore");
    scrollSentinel.appendChild(span);
    return;
  }

  if (errorMsg) {
    const span = document.createElement("span");
    span.textContent = errorMsg;
    const retryBtn = document.createElement("button");
    retryBtn.type = "button";
    retryBtn.className = "load-more-btn";
    retryBtn.textContent = t(currentLocale, "results.retry");
    retryBtn.addEventListener("click", () => void loadNextPage());
    scrollSentinel.append(span, retryBtn);
    return;
  }

  if (nextCursor) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "load-more-btn";
    btn.textContent = t(currentLocale, "results.loadMore");
    btn.addEventListener("click", () => void loadNextPage());
    scrollSentinel.appendChild(btn);
  } else {
    const span = document.createElement("span");
    span.textContent = t(currentLocale, "results.allShown");
    scrollSentinel.appendChild(span);
  }
}

function updateResultsSummary(): void {
  if (lastSummaryMode === "ready") {
    resultsSummary.textContent = t(currentLocale, "results.ready");
    return;
  }
  if (lastSummaryMode === "searching") {
    resultsSummary.textContent = t(currentLocale, "results.searching");
    return;
  }
  if (lastSummaryMode === "error") {
    resultsSummary.textContent = t(currentLocale, "results.searchError");
    return;
  }
  if (currentItems.length === 0) {
    resultsSummary.textContent = t(currentLocale, "results.noneProvider", {
      provider: currentProvider,
    });
    return;
  }
  const sources = Array.from(loadedProviders).join(" + ") || currentProvider;
  resultsSummary.textContent = t(currentLocale, "results.loaded", {
    count: currentItems.length,
    sources,
  });
}

function renderResultsList(): void {
  resultsList.innerHTML = "";
  cardWaveCanvases.clear();

  if (currentItems.length === 0) {
    const empty = document.createElement("div");
    empty.className = "state-card";
    const strong = document.createElement("strong");
    strong.textContent = t(currentLocale, "results.emptyTitle");
    const desc = document.createTextNode(t(currentLocale, "results.emptyDesc"));
    empty.append(strong, desc);
    resultsList.appendChild(empty);
    return;
  }

  for (const item of currentItems) {
    resultsList.appendChild(createSfxCardElement(item));
  }

  updateScrollSentinel();
  resultsList.appendChild(scrollSentinel);

  requestAnimationFrame(() => updatePlaybackUi());
}

function renderLoadingSkeletons(): void {
  resultsList.innerHTML = "";
  cardWaveCanvases.clear();
  for (let i = 0; i < 6; i++) {
    const sk = document.createElement("div");
    sk.className = "skeleton-card";
    resultsList.appendChild(sk);
  }
}

async function refreshFooterMeta(): Promise<void> {
  try {
    const [settings, stats] = await Promise.all([
      invoke<AppSettings>("get_settings"),
      invoke<CacheStats>("get_cache_stats"),
    ]);
    sidebarProvider.textContent =
      settings.providerMode === "mixkit" || !settings.freesoundApiKey.trim()
        ? "Mixkit"
        : "Freesound";
    lastCacheEntryCount = stats.entryCount;
    lastCacheStats = stats;
    sidebarCacheCount.textContent = t(currentLocale, "sidebar.cacheCount", {
      count: stats.entryCount,
    });
  } catch {
    // Non-fatal
  }
}

async function runSearch(queryOverride?: string): Promise<void> {
  const token = ++lastSearchToken;
  const query = (queryOverride ?? searchInput.value).trim();
  const minDuration = parseFloat(minDurInput.value);
  const maxDuration = parseFloat(maxDurInput.value);

  activeSearchQuery = query;
  nextCursor = null;
  isLoadingMore = false;
  seenIds.clear();
  loadedProviders.clear();

  searchBtn.disabled = true;
  warningBanner.classList.add("hidden");
  lastSummaryMode = "searching";
  updateResultsSummary();
  renderLoadingSkeletons();

  try {
    const resp = await invoke<SearchResponse>("search_sfx", {
      request: {
        query,
        minDuration,
        maxDuration,
        envelope: userEnvelope ? normalizeEnvelope(userEnvelope) : null,
        cursor: null,
      },
    });

    if (token !== lastSearchToken) return;

    currentItems = [];
    drySnapshots.clear();

    for (const item of resp.items) {
      if (seenIds.has(item.id)) continue;
      seenIds.add(item.id);
      currentItems.push(item);
      drySnapshots.set(item.id, {
        envelope: [...item.envelope],
        duration: item.duration,
      });
    }

    currentProvider = resp.providerUsed;
    loadedProviders.add(resp.providerUsed);
    nextCursor = resp.nextCursor;
    sidebarProvider.textContent = currentProvider;

    if (currentItems.length > 0) {
      selectedSoundId = currentItems[0].id;
      fxActiveSound.textContent = currentItems[0].title;
    } else {
      selectedSoundId = null;
      fxActiveSound.textContent = t(currentLocale, "fx.selectSound");
    }

    if (resp.warning) {
      warningBanner.textContent = resp.warning;
      warningBanner.classList.remove("hidden");
    }

    lastSummaryMode = "results";
    updateResultsSummary();
    renderResultsList();

    if (selectedSoundId && !isFxClean(fxParams)) {
      void updateFxWaveformNow();
    } else {
      renderEnvelopeCanvas();
    }
    void refreshFooterMeta();

    // If page 1 is short enough not to overflow the list, prefetch the next batch automatically.
    if (nextCursor && resultsList.scrollHeight <= resultsList.clientHeight + 80) {
      void loadNextPage();
    }
  } catch (err) {
    if (token !== lastSearchToken) return;
    currentItems = [];
    selectedSoundId = null;
    nextCursor = null;
    fxActiveSound.textContent = t(currentLocale, "fx.selectSound");
    resultsList.innerHTML = "";
    const errCard = document.createElement("div");
    errCard.className = "state-card";
    const strong = document.createElement("strong");
    strong.textContent = t(currentLocale, "results.errorTitle");
    const errText = document.createTextNode(String(err));
    errCard.append(strong, errText);
    resultsList.appendChild(errCard);
    lastSummaryMode = "error";
    updateResultsSummary();
  } finally {
    if (token === lastSearchToken) {
      searchBtn.disabled = false;
    }
  }
}

async function loadNextPage(emptyStreak = 0): Promise<void> {
  if (!nextCursor || isLoadingMore) return;

  const token = lastSearchToken;
  const cursorToLoad = nextCursor;
  const minDuration = parseFloat(minDurInput.value);
  const maxDuration = parseFloat(maxDurInput.value);

  isLoadingMore = true;
  updateScrollSentinel();

  try {
    const resp = await invoke<SearchResponse>("search_sfx", {
      request: {
        query: activeSearchQuery,
        minDuration,
        maxDuration,
        envelope: userEnvelope ? normalizeEnvelope(userEnvelope) : null,
        cursor: cursorToLoad,
      },
    });

    if (token !== lastSearchToken) return;

    nextCursor = resp.nextCursor;
    loadedProviders.add(resp.providerUsed);

    const newItems: SfxItem[] = [];
    for (const item of resp.items) {
      if (seenIds.has(item.id)) continue;
      seenIds.add(item.id);
      currentItems.push(item);
      drySnapshots.set(item.id, {
        envelope: [...item.envelope],
        duration: item.duration,
      });
      newItems.push(item);
    }

    for (const item of newItems) {
      const card = createSfxCardElement(item);
      resultsList.insertBefore(card, scrollSentinel);
    }

    isLoadingMore = false;
    updateResultsSummary();
    updateScrollSentinel(resp.warning ?? undefined);
    requestAnimationFrame(() => updatePlaybackUi());
    void refreshFooterMeta();

    if (newItems.length === 0 && nextCursor && !resp.warning && emptyStreak < 2) {
      void loadNextPage(emptyStreak + 1);
    }
  } catch (err) {
    if (token !== lastSearchToken) return;
    isLoadingMore = false;
    updateScrollSentinel(String(err));
  }
}

const sentinelObserver = new IntersectionObserver(
  (entries) => {
    if (entries.some((entry) => entry.isIntersecting) && nextCursor && !isLoadingMore) {
      void loadNextPage();
    }
  },
  {
    root: resultsList,
    rootMargin: "280px 0px",
  }
);
sentinelObserver.observe(scrollSentinel);

resultsList.addEventListener("scroll", () => {
  if (!nextCursor || isLoadingMore) return;
  const remaining = resultsList.scrollHeight - resultsList.scrollTop - resultsList.clientHeight;
  if (remaining < 260) {
    void loadNextPage();
  }
});

// FX Rack listeners
toggleFxBtn.addEventListener("click", () => {
  const collapsed = fxRack.classList.toggle("collapsed");
  toggleFxBtn.classList.toggle("active", !collapsed);
  renderEnvelopeCanvas();
  updatePlaybackUi();
});

fxPresetButtons.forEach((btn) => {
  btn.addEventListener("click", () => {
    const preset = btn.dataset.fxPreset;
    if (preset) applyFxPreset(preset);
  });
});

fxResetBtn.addEventListener("click", () => applyFxPreset("clean"));

fxPreviewBtn.addEventListener("click", () => {
  const targetId = selectedSoundId ?? currentItems[0]?.id;
  if (!targetId) {
    showToast(t(currentLocale, "fx.noSoundToast"));
    return;
  }
  const item = currentItems.find((x) => x.id === targetId);
  if (item) {
    void togglePlay(item, 0);
  }
});

fxSliders.forEach((slider) => {
  slider.addEventListener("input", () => {
    readFxParamsFromControls();
    scheduleFxWaveformUpdate();
  });
  slider.addEventListener("dblclick", () => {
    const def = slider.dataset.default;
    if (def !== undefined) {
      slider.value = def;
      readFxParamsFromControls();
      scheduleFxWaveformUpdate();
    }
  });
});

[fxReverseInput, fxNormalizeInput].forEach((checkbox) => {
  checkbox.addEventListener("change", () => {
    readFxParamsFromControls();
    scheduleFxWaveformUpdate();
  });
});

function renderCacheStatsText(): void {
  if (!lastCacheStats) {
    cacheStatsText.textContent = t(currentLocale, "settings.loading");
    return;
  }
  cacheStatsText.textContent = t(currentLocale, "settings.cacheStats", {
    entries: lastCacheStats.entryCount,
    size: formatBytes(lastCacheStats.dbSizeBytes),
    wavs: lastCacheStats.tempWavCount,
  });
}

function setLocale(locale: Locale): void {
  currentLocale = locale;
  applyDomTranslations(locale);

  syncDurationSlider("min");
  renderEnvelopeCanvas();
  updateResultsSummary();
  updateScrollSentinel();

  const selectedItem = selectedSoundId
    ? currentItems.find((x) => x.id === selectedSoundId)
    : undefined;
  fxActiveSound.textContent = selectedItem
    ? selectedItem.title
    : t(currentLocale, "fx.selectSound");

  sidebarCacheCount.textContent = t(currentLocale, "sidebar.cacheCount", {
    count: lastCacheEntryCount,
  });
  toggleKeyBtn.textContent = t(
    currentLocale,
    apiKeyInput.type === "password" ? "settings.show" : "settings.hide"
  );
  renderCacheStatsText();

  // Refresh dynamic labels inside already-rendered sound cards
  resultsList.querySelectorAll<HTMLElement>(".sfx-card").forEach((card) => {
    const id = card.dataset.soundId ?? "";
    const item = currentItems.find((x) => x.id === id);
    if (!item) return;

    const durEl = card.querySelector<HTMLElement>(".sfx-duration");
    if (durEl) durEl.textContent = formatDuration(item.duration);

    const playBtn = card.querySelector<HTMLButtonElement>(".play-btn");
    if (playBtn) playBtn.title = t(currentLocale, "card.playTitle");

    const waveWrap = card.querySelector<HTMLElement>(".wave-wrap");
    if (waveWrap) waveWrap.title = t(currentLocale, "card.seekTitle");

    const badge = card.querySelector<HTMLElement>(".match-badge");
    if (badge) badge.title = t(currentLocale, "card.matchTitle");

    const wavBtn = card.querySelector<HTMLButtonElement>(".wav-btn");
    if (wavBtn) wavBtn.title = t(currentLocale, "card.wavTitle");
  });

  if (currentItems.length === 0 && lastSummaryMode === "results") {
    renderResultsList();
  }
}

// Settings modal logic
async function refreshCacheStats(): Promise<void> {
  try {
    const stats = await invoke<CacheStats>("get_cache_stats");
    lastCacheStats = stats;
    lastCacheEntryCount = stats.entryCount;
    renderCacheStatsText();
    sidebarCacheCount.textContent = t(currentLocale, "sidebar.cacheCount", {
      count: stats.entryCount,
    });
  } catch (err) {
    cacheStatsText.textContent = String(err);
  }
}

async function openSettingsModal(): Promise<void> {
  try {
    const settings = await invoke<AppSettings>("get_settings");
    apiKeyInput.value = settings.freesoundApiKey;
    providerModeSelect.value = settings.providerMode;
    savedLocale = normalizeLocale(settings.language);
    languageSelect.value = savedLocale;
    if (currentLocale !== savedLocale) {
      setLocale(savedLocale);
    }
  } catch (err) {
    showToast(String(err));
  }
  await refreshCacheStats();
  settingsModal.classList.remove("hidden");
}

function closeSettingsModal(revertLanguage = true): void {
  settingsModal.classList.add("hidden");
  if (revertLanguage && currentLocale !== savedLocale) {
    setLocale(savedLocale);
    languageSelect.value = savedLocale;
  }
}

openSettingsBtn.addEventListener("click", () => void openSettingsModal());
closeSettingsBtn.addEventListener("click", () => closeSettingsModal(true));
cancelSettingsBtn.addEventListener("click", () => closeSettingsModal(true));

languageSelect.addEventListener("change", () => {
  setLocale(normalizeLocale(languageSelect.value));
});

toggleKeyBtn.addEventListener("click", () => {
  const isPass = apiKeyInput.type === "password";
  apiKeyInput.type = isPass ? "text" : "password";
  toggleKeyBtn.textContent = t(currentLocale, isPass ? "settings.hide" : "settings.show");
});

saveSettingsBtn.addEventListener("click", async () => {
  const chosenLocale = normalizeLocale(languageSelect.value);
  const settings: AppSettings = {
    freesoundApiKey: apiKeyInput.value.trim(),
    providerMode: providerModeSelect.value as ProviderMode,
    language: chosenLocale,
  };
  try {
    await invoke("save_settings", { settings });
    savedLocale = chosenLocale;
    setLocale(chosenLocale);
    sidebarProvider.textContent =
      settings.providerMode === "mixkit" || !settings.freesoundApiKey
        ? "Mixkit"
        : "Freesound";
    closeSettingsModal(false);
    showToast(t(currentLocale, "toast.saved"));
  } catch (err) {
    showToast(t(currentLocale, "toast.saveErr", { err: String(err) }));
  }
});

clearCacheBtn.addEventListener("click", async () => {
  try {
    const removed = await invoke<number>("clear_cache");
    preparedWavPaths.clear();
    await refreshCacheStats();
    showToast(t(currentLocale, "toast.cacheCleared", { count: removed }));
  } catch (err) {
    showToast(String(err));
  }
});

// Window edge resize dragging
resizeHandles.forEach((handle) => {
  handle.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    const dir = handle.dataset.resize as WindowResizeDirection | undefined;
    if (dir) {
      void appWindow.startResizeDragging(dir);
    }
  });
});

// Window & keyboard controls
searchForm.addEventListener("submit", (e) => {
  e.preventDefault();
  void runSearch();
});

minimizeBtn.addEventListener("click", () => {
  void appWindow.minimize();
});

closeBtn.addEventListener("click", () => {
  void appWindow.close();
});

window.addEventListener("keydown", (e) => {
  if ((e.ctrlKey || e.metaKey) && e.code === "KeyK") {
    e.preventDefault();
    searchInput.focus();
    searchInput.select();
    return;
  }
  if (e.code === "Escape" && !settingsModal.classList.contains("hidden")) {
    closeSettingsModal();
    return;
  }
  if (
    e.code === "Space" &&
    document.activeElement?.tagName !== "INPUT" &&
    document.activeElement?.tagName !== "SELECT"
  ) {
    e.preventDefault();
    if (playback.isPlaying) {
      void invoke("stop_sfx").then(() => {
        playback = { ...playback, isPlaying: false };
        updatePlaybackUi();
        renderEnvelopeCanvas();
      });
    } else if (currentItems.length > 0) {
      const target =
        currentItems.find((x) => x.id === (playback.soundId ?? selectedSoundId)) ??
        currentItems[0];
      void togglePlay(target);
    }
  }
});

window.addEventListener("resize", () => {
  renderEnvelopeCanvas();
  updatePlaybackUi();
});

// Initialization
async function init(): Promise<void> {
  try {
    const settings = await invoke<AppSettings>("get_settings");
    savedLocale = normalizeLocale(settings.language);
    languageSelect.value = savedLocale;
    setLocale(savedLocale);
  } catch {
    setLocale("ru");
  }

  syncDurationSlider("min");
  syncFxControlsFromParams();
  renderEnvelopeCanvas();
  void refreshFooterMeta();

  searchInput.value = "whoosh";
  syncCategoryHighlight("whoosh");
  void runSearch("whoosh");
}

void init();
