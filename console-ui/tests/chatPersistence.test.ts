import { describe, expect, it, vi } from 'vitest';
import type { ChatSession } from '../src/domain/chat/types';
import {
  createSaveScheduler,
  loadChatSnapshot,
  saveChatSnapshot,
} from '../src/domain/chat/store/chatPersistence';

class MemoryStorage implements Storage {
  private data = new Map<string, string>();
  get length() {
    return this.data.size;
  }
  clear() {
    this.data.clear();
  }
  getItem(key: string) {
    return this.data.get(key) ?? null;
  }
  key(index: number) {
    return Array.from(this.data.keys())[index] ?? null;
  }
  removeItem(key: string) {
    this.data.delete(key);
  }
  setItem(key: string, value: string) {
    this.data.set(key, value);
  }
}

function buildSession(overrides: Partial<ChatSession> = {}): ChatSession {
  return {
    id: 's1',
    provider: 'openai',
    title: 'Test',
    createdAt: 1,
    updatedAt: 2,
    messages: [],
    totalTokens: 0,
    status: 'idle',
    ...overrides,
  };
}

describe('chatPersistence', () => {
  it('returns null when storage is empty', () => {
    const storage = new MemoryStorage();
    expect(loadChatSnapshot(storage)).toBeNull();
  });

  it('normalizes provider when loading', () => {
    const storage = new MemoryStorage();
    const sessions: ChatSession[] = [
      buildSession({ id: 'acp', provider: 'acp' }),
      buildSession({ id: 'other', provider: 'openai' }),
      buildSession({ id: 'unknown', provider: 'openai' }),
    ];
    storage.setItem(
      'octovalve-chat-sessions',
      JSON.stringify({ sessions: [{ ...sessions[0] }, { ...sessions[1] }, { ...sessions[2], provider: 'other' }], activeSessionId: 'acp' })
    );

    const snapshot = loadChatSnapshot(storage);
    expect(snapshot?.sessions[0]?.provider).toBe('acp');
    expect(snapshot?.sessions[1]?.provider).toBe('openai');
    expect(snapshot?.sessions[2]?.provider).toBe('openai');
    expect(snapshot?.activeSessionId).toBe('acp');
  });

  it('sanitizes images when saving', () => {
    const storage = new MemoryStorage();
    const sessions: ChatSession[] = [
      buildSession({
        messages: [
          {
            id: 'm1',
            ts: 1,
            type: 'say',
            role: 'user',
            content: 'hi',
            status: 'complete',
            images: ['data:image/png;base64,abc'],
          },
        ],
      }),
    ];

    saveChatSnapshot(storage, { sessions, activeSessionId: 's1' });
    const raw = storage.getItem('octovalve-chat-sessions');
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw as string);
    expect(parsed.sessions[0].messages[0].images).toBeUndefined();
  });

  it('schedules saves only once per interval', () => {
    vi.useFakeTimers();
    const saveFn = vi.fn();
    const scheduler = createSaveScheduler(saveFn, 200);

    scheduler.schedule();
    scheduler.schedule();
    expect(saveFn).not.toHaveBeenCalled();

    vi.advanceTimersByTime(199);
    expect(saveFn).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(saveFn).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });
});
