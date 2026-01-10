import type { ChatSession } from '../types';

export const CHAT_SESSIONS_STORAGE_KEY = 'octovalve-chat-sessions';

export type ChatStorageSnapshot = {
  sessions: ChatSession[];
  activeSessionId: string | null;
};

function normalizeProvider(value: unknown): 'acp' | 'openai' {
  return value === 'acp' ? 'acp' : 'openai';
}

export function sanitizeSessionsForStorage(sessions: ChatSession[]): ChatSession[] {
  return sessions.map((session) => ({
    ...session,
    messages: session.messages.map((message) => ({
      ...message,
      images: undefined,
    })),
  }));
}

export function loadChatSnapshot(storage: Storage, key = CHAT_SESSIONS_STORAGE_KEY): ChatStorageSnapshot | null {
  try {
    const stored = storage.getItem(key);
    if (!stored) {
      return null;
    }
    const data = JSON.parse(stored) as { sessions?: ChatSession[]; activeSessionId?: string | null };
    const rawSessions = Array.isArray(data.sessions) ? data.sessions : [];
    const sessions = rawSessions.map((session) => ({
      ...session,
      provider: normalizeProvider((session as ChatSession).provider),
    }));
    const activeSessionId = typeof data.activeSessionId === 'string' ? data.activeSessionId : null;
    return { sessions, activeSessionId };
  } catch (e) {
    console.warn('Failed to load chat sessions from storage:', e);
    return null;
  }
}

export function saveChatSnapshot(
  storage: Storage,
  snapshot: ChatStorageSnapshot,
  key = CHAT_SESSIONS_STORAGE_KEY
): void {
  try {
    const sanitizedSessions = sanitizeSessionsForStorage(snapshot.sessions);
    storage.setItem(
      key,
      JSON.stringify({
        sessions: sanitizedSessions,
        activeSessionId: snapshot.activeSessionId,
      })
    );
  } catch (e) {
    console.warn('Failed to save chat sessions to storage:', e);
  }
}

export function createSaveScheduler(saveFn: () => void, delayMs = 400) {
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  return {
    schedule() {
      if (saveTimer) {
        return;
      }
      saveTimer = setTimeout(() => {
        saveTimer = null;
        saveFn();
      }, delayMs);
    },
    flush() {
      if (saveTimer) {
        clearTimeout(saveTimer);
        saveTimer = null;
      }
      saveFn();
    },
    cancel() {
      if (saveTimer) {
        clearTimeout(saveTimer);
        saveTimer = null;
      }
    },
  };
}
