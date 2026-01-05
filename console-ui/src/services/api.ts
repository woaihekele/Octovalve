import { listen } from '@tauri-apps/api/event';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { i18n } from '../i18n';
import type {
  AiRiskApiResponse,
  ConfigFilePayload,
  ConsoleEvent,
  ProfilesStatus,
  ServiceSnapshot,
  TargetInfo,
} from '../shared/types';

const DEFAULT_HTTP = 'http://127.0.0.1:19309';
const DEFAULT_WS = 'ws://127.0.0.1:19309/ws';

const TAURI_AVAILABLE = isTauri();
const t = i18n.global.t;
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

export type StartupCheckResult = {
  ok: boolean;
  needs_setup: boolean;
  errors: string[];
  proxy_path: string;
  broker_path: string;
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

export type ConsoleLogChunk = {
  content: string;
  nextOffset: number;
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

export async function cancelCommand(name: string, id: string) {
  if (TAURI_AVAILABLE) {
    await invoke('proxy_cancel', { name, id });
    return;
  }
  const response = await fetch(joinUrl(HTTP_BASE, `/targets/${encodeURIComponent(name)}/cancel`), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ id }),
  });
  if (!response.ok) {
    throw new Error(`cancel failed: ${response.status}`);
  }
}

export async function getProxyConfigStatus(): Promise<ProxyConfigStatus> {
  if (TAURI_AVAILABLE) {
    return invoke<ProxyConfigStatus>('get_proxy_config_status');
  }
  return { present: true, path: '', example_path: '' };
}

export async function listProfiles(): Promise<ProfilesStatus> {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.profiles'));
  }
  return invoke<ProfilesStatus>('list_profiles');
}

export async function createProfile(name: string) {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.profiles'));
  }
  await invoke('create_profile', { name });
}

export async function deleteProfile(name: string) {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.profiles'));
  }
  await invoke('delete_profile', { name });
}

export async function selectProfile(name: string) {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.profiles'));
  }
  await invoke('select_profile', { name });
}

export async function readProfileProxyConfig(name: string): Promise<ConfigFilePayload> {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.profiles'));
  }
  return invoke<ConfigFilePayload>('read_profile_proxy_config', { name });
}

export async function writeProfileProxyConfig(name: string, content: string) {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.profiles'));
  }
  await invoke('write_profile_proxy_config', { name, content });
}

export async function readProfileBrokerConfig(name: string): Promise<ConfigFilePayload> {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.profiles'));
  }
  return invoke<ConfigFilePayload>('read_profile_broker_config', { name });
}

export async function writeProfileBrokerConfig(name: string, content: string) {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.profiles'));
  }
  await invoke('write_profile_broker_config', { name, content });
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

export async function readProxyConfig(): Promise<ConfigFilePayload> {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.configEditor'));
  }
  return invoke<ConfigFilePayload>('read_proxy_config');
}

export async function writeProxyConfig(content: string) {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.configEditor'));
  }
  await invoke('write_proxy_config', { content });
}

export async function readBrokerConfig(): Promise<ConfigFilePayload> {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.configEditor'));
  }
  return invoke<ConfigFilePayload>('read_broker_config');
}

export async function writeBrokerConfig(content: string) {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.configEditor'));
  }
  await invoke('write_broker_config', { content });
}

export async function restartConsole() {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.configEditor'));
  }
  await invoke('restart_console');
}

export async function validateStartupConfig(): Promise<StartupCheckResult> {
  if (!TAURI_AVAILABLE) {
    return { ok: true, needs_setup: false, errors: [], proxy_path: '', broker_path: '' };
  }
  return invoke<StartupCheckResult>('validate_startup_config');
}

export async function reloadRemoteBrokers() {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.configEditor'));
  }
  await invoke('proxy_reload_remote_brokers');
}

export async function readConsoleLog(offset: number, maxBytes: number): Promise<ConsoleLogChunk> {
  if (!TAURI_AVAILABLE) {
    return { content: '', nextOffset: 0 };
  }
  return invoke<ConsoleLogChunk>('read_console_log', { offset, maxBytes });
}

export async function terminalOpen(name: string, cols: number, rows: number, term?: string) {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.terminal'));
  }
  return invoke<string>('terminal_open', { name, cols, rows, term });
}

export async function terminalInput(sessionId: string, dataBase64: string) {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.terminal'));
  }
  await invoke('terminal_input', { sessionId, dataBase64 });
}

export async function terminalResize(sessionId: string, cols: number, rows: number) {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.terminal'));
  }
  await invoke('terminal_resize', { sessionId, cols, rows });
}

export async function terminalClose(sessionId: string) {
  if (!TAURI_AVAILABLE) {
    return;
  }
  await invoke('terminal_close', { sessionId });
}

export type AiRiskRequestPayload = {
  base_url: string;
  chat_path: string;
  model: string;
  api_key: string;
  prompt: string;
  timeout_ms?: number;
};

export async function aiRiskAssess(request: AiRiskRequestPayload): Promise<AiRiskApiResponse> {
  if (!TAURI_AVAILABLE) {
    throw new Error(t('api.tauriOnly.aiRisk'));
  }
  return invoke<AiRiskApiResponse>('ai_risk_assess', { request });
}
