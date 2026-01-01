import { defineStore } from 'pinia';
import { ref, computed } from 'vue';
import type { ChatSession, ChatMessage, ChatState, SendMessageOptions, ChatConfig } from '../types';

function generateId(): string {
  return `${Date.now()}-${Math.random().toString(36).slice(2, 11)}`;
}

export const useChatStore = defineStore('chat', () => {
  // State
  const sessions = ref<ChatSession[]>([]);
  const activeSessionId = ref<string | null>(null);
  const isConnected = ref(true);
  const isStreaming = ref(false);
  const error = ref<string | null>(null);
  const config = ref<ChatConfig>({
    greeting: '你好，我是 AI 助手。请描述你想要完成的任务。',
  });

  // Getters
  const activeSession = computed(() =>
    sessions.value.find((s) => s.id === activeSessionId.value) ?? null
  );

  const messages = computed(() => activeSession.value?.messages ?? []);

  const lastMessage = computed(() => messages.value.at(-1) ?? null);

  const canSend = computed(() => !isStreaming.value && isConnected.value);

  // Actions
  function createSession(title?: string): ChatSession {
    const session: ChatSession = {
      id: generateId(),
      title: title ?? `会话 ${sessions.value.length + 1}`,
      createdAt: Date.now(),
      updatedAt: Date.now(),
      messages: [],
      totalTokens: 0,
      status: 'idle',
    };
    sessions.value.push(session);
    activeSessionId.value = session.id;
    return session;
  }

  function setActiveSession(sessionId: string | null) {
    activeSessionId.value = sessionId;
  }

  function deleteSession(sessionId: string) {
    const index = sessions.value.findIndex((s) => s.id === sessionId);
    if (index !== -1) {
      sessions.value.splice(index, 1);
      if (activeSessionId.value === sessionId) {
        activeSessionId.value = sessions.value[0]?.id ?? null;
      }
    }
  }

  function addMessage(message: Omit<ChatMessage, 'id' | 'ts'>): ChatMessage {
    if (!activeSession.value) {
      createSession();
    }
    const newMessage: ChatMessage = {
      ...message,
      id: generateId(),
      ts: Date.now(),
    };
    activeSession.value!.messages.push(newMessage);
    activeSession.value!.updatedAt = Date.now();
    return newMessage;
  }

  function updateMessage(messageId: string, updates: Partial<ChatMessage>) {
    if (!activeSession.value) return;
    const message = activeSession.value.messages.find((m) => m.id === messageId);
    if (message) {
      Object.assign(message, updates);
      activeSession.value.updatedAt = Date.now();
    }
  }

  function appendToMessage(messageId: string, content: string) {
    if (!activeSession.value) return;
    const message = activeSession.value.messages.find((m) => m.id === messageId);
    if (message) {
      message.content += content;
      message.partial = true;
      activeSession.value.updatedAt = Date.now();
    }
  }

  function setStreaming(value: boolean) {
    isStreaming.value = value;
    if (activeSession.value) {
      activeSession.value.status = value ? 'running' : 'idle';
    }
  }

  function setConnected(value: boolean) {
    isConnected.value = value;
  }

  function setError(message: string | null) {
    error.value = message;
  }

  function clearMessages() {
    if (activeSession.value) {
      activeSession.value.messages = [];
      activeSession.value.updatedAt = Date.now();
    }
  }

  function setConfig(newConfig: Partial<ChatConfig>) {
    config.value = { ...config.value, ...newConfig };
  }

  // Persistence
  function loadFromStorage() {
    try {
      const stored = localStorage.getItem('octovalve-chat-sessions');
      if (stored) {
        const data = JSON.parse(stored);
        sessions.value = data.sessions ?? [];
        activeSessionId.value = data.activeSessionId ?? null;
      }
    } catch (e) {
      console.warn('Failed to load chat sessions from storage:', e);
    }
  }

  function saveToStorage() {
    try {
      localStorage.setItem(
        'octovalve-chat-sessions',
        JSON.stringify({
          sessions: sessions.value,
          activeSessionId: activeSessionId.value,
        })
      );
    } catch (e) {
      console.warn('Failed to save chat sessions to storage:', e);
    }
  }

  return {
    // State
    sessions,
    activeSessionId,
    isConnected,
    isStreaming,
    error,
    config,
    // Getters
    activeSession,
    messages,
    lastMessage,
    canSend,
    // Actions
    createSession,
    setActiveSession,
    deleteSession,
    addMessage,
    updateMessage,
    appendToMessage,
    setStreaming,
    setConnected,
    setError,
    clearMessages,
    setConfig,
    loadFromStorage,
    saveToStorage,
  };
});
