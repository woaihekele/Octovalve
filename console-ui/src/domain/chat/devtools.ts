import { invoke, isTauri } from '@tauri-apps/api/core';
import { unref } from 'vue';

import type { ChatSession } from './types';
import { CHAT_SESSIONS_STORAGE_KEY, type ChatStorageSnapshot } from './store/chatPersistence';

type ImportOptions = {
  force?: boolean;
};

type ChatStoreLike = {
  sessions: ChatSession[] | { value: ChatSession[] };
  activeSessionId: string | null | { value: string | null };
  isStreaming: boolean | { value: boolean };
  loadFromStorage: () => void;
  setActiveSession: (sessionId: string | null) => void;
};

function ensureSnapshot(value: unknown): ChatStorageSnapshot {
  if (!value || typeof value !== 'object') {
    throw new Error('无效的会话快照（不是对象）');
  }
  const raw = value as { sessions?: unknown; activeSessionId?: unknown };
  if (!Array.isArray(raw.sessions)) {
    throw new Error('无效的会话快照（sessions 不是数组）');
  }
  const activeSessionId =
    typeof raw.activeSessionId === 'string' || raw.activeSessionId === null
      ? raw.activeSessionId
      : null;
  return { sessions: raw.sessions as ChatSession[], activeSessionId };
}

function parseSnapshot(text: string): ChatStorageSnapshot {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch (err) {
    throw new Error(`JSON 解析失败：${String(err)}`);
  }
  if (typeof parsed === 'string') {
    try {
      return ensureSnapshot(JSON.parse(parsed));
    } catch (err) {
      throw new Error(`二次解析失败：${String(err)}`);
    }
  }
  return ensureSnapshot(parsed);
}

function resolveRef<T>(value: T | { value: T }): T {
  return unref(value as T);
}

export function registerChatImportCommand(store: ChatStoreLike) {
  if (typeof window === 'undefined') {
    return;
  }
  const root = (window as any).__octovalve || ((window as any).__octovalve = {});

  root.importChatSnapshot = async (path: string, options: ImportOptions = {}) => {
    if (!isTauri()) {
      throw new Error('仅支持 Tauri 环境');
    }
    const trimmed = String(path || '').trim();
    if (!trimmed) {
      throw new Error('请输入有效的文件路径');
    }

    const isStreaming = resolveRef(store.isStreaming);
    if (isStreaming) {
      throw new Error('当前正在生成中，请先停止');
    }

    const existingSessions = resolveRef(store.sessions) || [];
    const hasContent = existingSessions.some((session) => (session.messages?.length ?? 0) > 0);
    if (hasContent && !options.force) {
      throw new Error('当前会话非空，请传入 { force: true } 进行覆盖');
    }

    const text = await invoke<string>('read_text_file', { path: trimmed });
    const snapshot = parseSnapshot(text);
    if (!snapshot.sessions.length) {
      throw new Error('快照里没有会话可恢复');
    }
    let activeSessionId = snapshot.activeSessionId;
    if (!activeSessionId || !snapshot.sessions.some((s) => s.id === activeSessionId)) {
      activeSessionId = snapshot.sessions[0].id;
    }

    const payload: ChatStorageSnapshot = {
      sessions: snapshot.sessions,
      activeSessionId,
    };
    window.localStorage?.setItem(CHAT_SESSIONS_STORAGE_KEY, JSON.stringify(payload));
    store.loadFromStorage();
    const updatedSessions = resolveRef(store.sessions) || [];
    const updatedActive = resolveRef(store.activeSessionId);
    if (!updatedActive && updatedSessions.length > 0) {
      store.setActiveSession(updatedSessions[0].id);
    }
    return {
      sessions: updatedSessions.length,
      activeSessionId: resolveRef(store.activeSessionId),
    };
  };
}
