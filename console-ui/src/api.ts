import { invoke } from '@tauri-apps/api/tauri';
import type { ConsoleEvent, ServiceSnapshot, TargetInfo } from './types';

const DEFAULT_HTTP = 'http://127.0.0.1:19309';
const DEFAULT_WS = 'ws://127.0.0.1:19309/ws';

const HTTP_BASE = (import.meta.env.VITE_CONSOLE_HTTP as string | undefined) || DEFAULT_HTTP;
const WS_BASE = (import.meta.env.VITE_CONSOLE_WS as string | undefined) || DEFAULT_WS;
const TAURI_AVAILABLE =
  typeof window !== 'undefined' && typeof (window as { __TAURI__?: unknown }).__TAURI__ !== 'undefined';

function joinUrl(base: string, path: string) {
  const normalizedBase = base.endsWith('/') ? base.slice(0, -1) : base;
  const normalizedPath = path.startsWith('/') ? path : `/${path}`;
  return `${normalizedBase}${normalizedPath}`;
}

function resolveWsUrl(base: string) {
  if (base.startsWith('ws://') || base.startsWith('wss://')) {
    return base;
  }
  if (base.startsWith('http://')) {
    return `ws://${base.slice('http://'.length)}`;
  }
  if (base.startsWith('https://')) {
    return `wss://${base.slice('https://'.length)}`;
  }
  if (base.startsWith('/')) {
    const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws';
    return `${scheme}://${window.location.host}${base}`;
  }
  return base;
}

export async function fetchTargets(): Promise<TargetInfo[]> {
  const response = await fetch(joinUrl(HTTP_BASE, '/targets'));
  if (!response.ok) {
    throw new Error(`failed to fetch targets: ${response.status}`);
  }
  return response.json() as Promise<TargetInfo[]>;
}

export async function fetchSnapshot(name: string): Promise<ServiceSnapshot> {
  const response = await fetch(joinUrl(HTTP_BASE, `/targets/${encodeURIComponent(name)}/snapshot`));
  if (!response.ok) {
    throw new Error(`failed to fetch snapshot: ${response.status}`);
  }
  return response.json() as Promise<ServiceSnapshot>;
}

export async function approveCommand(name: string, id: string) {
  const response = await fetch(joinUrl(HTTP_BASE, `/targets/${encodeURIComponent(name)}/approve`), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ id }),
  });
  if (!response.ok) {
    throw new Error(`approve failed: ${response.status}`);
  }
}

export async function denyCommand(name: string, id: string) {
  const response = await fetch(joinUrl(HTTP_BASE, `/targets/${encodeURIComponent(name)}/deny`), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ id }),
  });
  if (!response.ok) {
    throw new Error(`deny failed: ${response.status}`);
  }
}

export function openConsoleSocket(onEvent: (event: ConsoleEvent) => void) {
  const ws = new WebSocket(resolveWsUrl(WS_BASE));
  ws.onmessage = (message) => {
    try {
      const parsed = JSON.parse(message.data) as ConsoleEvent;
      if (parsed && typeof parsed.type === 'string') {
        onEvent(parsed);
      }
    } catch (err) {
      console.warn('failed to parse websocket event', err);
    }
  };
  return ws;
}

export async function logUiEvent(message: string) {
  if (!TAURI_AVAILABLE) {
    return;
  }
  try {
    await invoke('log_ui_event', { message });
  } catch {
    // ignore logging failures
  }
}
