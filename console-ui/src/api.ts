import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/tauri';
import type { ConsoleEvent, ServiceSnapshot, TargetInfo } from './types';

const DEFAULT_HTTP = 'http://127.0.0.1:19309';
const DEFAULT_WS = 'ws://127.0.0.1:19309/ws';

const TAURI_AVAILABLE =
  typeof window !== 'undefined' && typeof (window as { __TAURI__?: unknown }).__TAURI__ !== 'undefined';
const RAW_HTTP = (import.meta.env.VITE_CONSOLE_HTTP as string | undefined) || DEFAULT_HTTP;
const RAW_WS = (import.meta.env.VITE_CONSOLE_WS as string | undefined) || DEFAULT_WS;
const HTTP_BASE = TAURI_AVAILABLE && RAW_HTTP.startsWith('/') ? DEFAULT_HTTP : RAW_HTTP;
const WS_BASE = TAURI_AVAILABLE && RAW_WS.startsWith('/') ? DEFAULT_WS : RAW_WS;

export type ConsoleConnectionStatus = 'connected' | 'connecting' | 'disconnected';
export type ConsoleStreamHandle = { close: () => void };
export type ProxyConfigStatus = {
  present: boolean;
  path: string;
  example_path: string;
};

export type TerminalOutputEvent = {
  session_id: string;
  data: string;
};

export type TerminalExitEvent = {
  session_id: string;
  code?: number | null;
};

export type TerminalErrorEvent = {
  session_id: string;
  message: string;
};

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
  if (TAURI_AVAILABLE) {
    return invoke<TargetInfo[]>('proxy_fetch_targets');
  }
  const response = await fetch(joinUrl(HTTP_BASE, '/targets'));
  if (!response.ok) {
    throw new Error(`failed to fetch targets: ${response.status}`);
  }
  return response.json() as Promise<TargetInfo[]>;
}

export async function fetchSnapshot(name: string): Promise<ServiceSnapshot> {
  if (TAURI_AVAILABLE) {
    return invoke<ServiceSnapshot>('proxy_fetch_snapshot', { name });
  }
  const response = await fetch(joinUrl(HTTP_BASE, `/targets/${encodeURIComponent(name)}/snapshot`));
  if (!response.ok) {
    throw new Error(`failed to fetch snapshot: ${response.status}`);
  }
  return response.json() as Promise<ServiceSnapshot>;
}

export async function approveCommand(name: string, id: string) {
  if (TAURI_AVAILABLE) {
    await invoke('proxy_approve', { name, id });
    return;
  }
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
  if (TAURI_AVAILABLE) {
    await invoke('proxy_deny', { name, id });
    return;
  }
  const response = await fetch(joinUrl(HTTP_BASE, `/targets/${encodeURIComponent(name)}/deny`), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ id }),
  });
  if (!response.ok) {
    throw new Error(`deny failed: ${response.status}`);
  }
}

export async function getProxyConfigStatus(): Promise<ProxyConfigStatus> {
  if (TAURI_AVAILABLE) {
    return invoke<ProxyConfigStatus>('get_proxy_config_status');
  }
  return { present: true, path: '', example_path: '' };
}

export async function openConsoleStream(
  onEvent: (event: ConsoleEvent) => void,
  onStatus?: (status: ConsoleConnectionStatus) => void
): Promise<ConsoleStreamHandle> {
  if (TAURI_AVAILABLE) {
    const unlistenEvent = await listen<ConsoleEvent>('console_event', (event) => {
      onEvent(event.payload);
    });
    const unlistenStatus = await listen<ConsoleConnectionStatus>('console_ws_status', (event) => {
      onStatus?.(event.payload);
    });
    await invoke('start_console_stream');
    return {
      close: () => {
        unlistenEvent();
        unlistenStatus();
      },
    };
  }

  let ws: WebSocket | null = null;
  let reconnectTimer: number | null = null;

  const connect = () => {
    onStatus?.('connecting');
    ws = new WebSocket(resolveWsUrl(WS_BASE));
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
    ws.onopen = () => {
      onStatus?.('connected');
    };
    ws.onclose = () => {
      onStatus?.('disconnected');
      scheduleReconnect();
    };
    ws.onerror = () => {
      onStatus?.('disconnected');
      scheduleReconnect();
    };
  };

  const scheduleReconnect = () => {
    if (reconnectTimer) {
      return;
    }
    reconnectTimer = window.setTimeout(() => {
      reconnectTimer = null;
      connect();
    }, 3000);
  };

  connect();

  return {
    close: () => {
      if (ws) {
        ws.close();
      }
      if (reconnectTimer) {
        window.clearTimeout(reconnectTimer);
      }
    },
  };
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

export async function terminalOpen(name: string, cols: number, rows: number, term?: string) {
  if (!TAURI_AVAILABLE) {
    throw new Error('terminal only available in Tauri');
  }
  return invoke<string>('terminal_open', { name, cols, rows, term });
}

export async function terminalInput(sessionId: string, dataBase64: string) {
  if (!TAURI_AVAILABLE) {
    throw new Error('terminal only available in Tauri');
  }
  await invoke('terminal_input', { sessionId, dataBase64 });
}

export async function terminalResize(sessionId: string, cols: number, rows: number) {
  if (!TAURI_AVAILABLE) {
    throw new Error('terminal only available in Tauri');
  }
  await invoke('terminal_resize', { sessionId, cols, rows });
}

export async function terminalClose(sessionId: string) {
  if (!TAURI_AVAILABLE) {
    return;
  }
  await invoke('terminal_close', { sessionId });
}
