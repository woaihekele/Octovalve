import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';

const TAURI_AVAILABLE = isTauri();

export async function startWindowDrag() {
  if (!TAURI_AVAILABLE) {
    return;
  }
  await getCurrentWindow().startDragging();
}

export async function setWindowMinSize(width: number, height: number) {
  if (!TAURI_AVAILABLE) {
    return;
  }
  const clampedWidth = Math.max(1, Math.round(width));
  const clampedHeight = Math.max(1, Math.round(height));
  await getCurrentWindow().setMinSize(new LogicalSize(clampedWidth, clampedHeight));
}

export async function setWindowSize(width: number, height: number) {
  if (!TAURI_AVAILABLE) {
    return;
  }
  const clampedWidth = Math.max(1, Math.round(width));
  const clampedHeight = Math.max(1, Math.round(height));
  await getCurrentWindow().setSize(new LogicalSize(clampedWidth, clampedHeight));
}

export async function getWindowLogicalSize() {
  if (!TAURI_AVAILABLE) {
    return null;
  }
  const appWindow = getCurrentWindow();
  const [size, scaleFactor] = await Promise.all([
    appWindow.innerSize(),
    appWindow.scaleFactor(),
  ]);
  const scale = Number.isFinite(scaleFactor) && scaleFactor > 0 ? scaleFactor : 1;
  return {
    width: size.width / scale,
    height: size.height / scale,
  };
}
