import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

const TAURI_AVAILABLE = isTauri();

export async function startWindowDrag() {
  if (!TAURI_AVAILABLE) {
    return;
  }
  await getCurrentWindow().startDragging();
}
