import { isTauri } from '@tauri-apps/api/core';

const UI_SCALE_MIN = 0.8;
const UI_SCALE_MAX = 1.5;

function clampScale(value: number): number {
  if (!Number.isFinite(value)) {
    return 1;
  }
  if (value < UI_SCALE_MIN) {
    return UI_SCALE_MIN;
  }
  if (value > UI_SCALE_MAX) {
    return UI_SCALE_MAX;
  }
  return value;
}

export async function applyUiScale(scale: number) {
  const clamped = clampScale(scale);
  if (isTauri()) {
    const { getCurrentWebview } = await import('@tauri-apps/api/webview');
    await getCurrentWebview().setZoom(clamped);
    return;
  }
  if (typeof document !== 'undefined') {
    document.documentElement.style.zoom = String(clamped);
  }
}
