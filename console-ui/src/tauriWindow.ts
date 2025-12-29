const TAURI_AVAILABLE =
  typeof window !== 'undefined' && typeof (window as { __TAURI__?: unknown }).__TAURI__ !== 'undefined';

export async function startWindowDrag() {
  if (!TAURI_AVAILABLE) {
    return;
  }
  const { appWindow } = await import('@tauri-apps/api/window');
  await appWindow.startDragging();
}
