import { defineStore } from 'pinia';
import { ref, computed, onUnmounted } from 'vue';
import type { ChatSession, ChatMessage, ChatState, SendMessageOptions, ChatConfig } from '../types';
import type { AuthMethod, AcpEvent, ContentDeltaPayload, ErrorPayload, CompletePayload } from '../services/acpService';
import { acpService } from '../services/acpService';

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

  // ACP state
  const acpInitialized = ref(false);
  const acpSessionId = ref<string | null>(null);
  const authMethods = ref<AuthMethod[]>([]);
  const currentAssistantMessageId = ref<string | null>(null);
  let acpEventUnlisten: (() => void) | null = null;

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

  // ACP Actions
  async function initializeAcp(cwd: string) {
    try {
      const response = await acpService.start(cwd);
      authMethods.value = response.authMethods;
      acpInitialized.value = true;
      setupAcpEventListener();
      return response;
    } catch (e) {
      setError(`Failed to initialize ACP: ${e}`);
      throw e;
    }
  }

  async function authenticateAcp(methodId: string) {
    try {
      await acpService.authenticate(methodId);
      setConnected(true);
    } catch (e) {
      setError(`Authentication failed: ${e}`);
      throw e;
    }
  }

  async function sendAcpMessage(content: string, cwd = '.') {
    if (!acpInitialized.value) {
      throw new Error('ACP not initialized');
    }

    // Create session if needed
    if (!acpSessionId.value) {
      const session = await acpService.newSession(cwd);
      acpSessionId.value = session.sessionId;
    }

    // Add user message
    addMessage({
      type: 'say',
      say: 'text',
      role: 'user',
      content,
      status: 'complete',
    });

    // Add assistant placeholder
    const assistantMsg = addMessage({
      type: 'say',
      say: 'text',
      role: 'assistant',
      content: '',
      status: 'streaming',
      partial: true,
    });
    currentAssistantMessageId.value = assistantMsg.id;

    setStreaming(true);

    try {
      await acpService.prompt(content);
    } catch (e) {
      updateMessage(assistantMsg.id, { status: 'error', content: `Error: ${e}` });
      setStreaming(false);
      currentAssistantMessageId.value = null;
      throw e;
    }
  }

  async function cancelAcp() {
    try {
      await acpService.cancel();
      setStreaming(false);
      if (currentAssistantMessageId.value) {
        updateMessage(currentAssistantMessageId.value, { status: 'cancelled' });
        currentAssistantMessageId.value = null;
      }
    } catch (e) {
      console.error('Failed to cancel:', e);
    }
  }

  async function stopAcp() {
    if (acpEventUnlisten) {
      acpEventUnlisten();
      acpEventUnlisten = null;
    }
    try {
      await acpService.stop();
    } catch (e) {
      console.error('Failed to stop ACP:', e);
    }
    acpInitialized.value = false;
    acpSessionId.value = null;
    setConnected(false);
  }

  function setupAcpEventListener() {
    acpService.onEvent(handleAcpEvent).then((unlisten) => {
      acpEventUnlisten = unlisten;
    });
  }

  function handleAcpEvent(event: AcpEvent) {
    const method = event.type;
    const payload = event.payload as Record<string, unknown>;

    if (method === 'session/update') {
      const update = payload.update as Record<string, unknown> | undefined;
      if (!update) return;
      
      const sessionUpdate = update.sessionUpdate as string | undefined;
      
      // Handle agent message chunks (AI response text)
      if (sessionUpdate === 'agent_message_chunk') {
        const content = update.content as { text?: string; type?: string } | undefined;
        if (content?.text && currentAssistantMessageId.value) {
          appendToMessage(currentAssistantMessageId.value, content.text);
        }
      }
      // Handle task complete
      else if (sessionUpdate === 'task_complete') {
        if (currentAssistantMessageId.value) {
          updateMessage(currentAssistantMessageId.value, { 
            status: 'complete', 
            partial: false 
          });
          currentAssistantMessageId.value = null;
        }
        setStreaming(false);
      }
      // Handle errors
      else if (sessionUpdate === 'error') {
        const errorMsg = (update.error as { message?: string })?.message || 'Unknown error';
        setError(errorMsg);
        if (currentAssistantMessageId.value) {
          updateMessage(currentAssistantMessageId.value, { 
            status: 'error',
            content: errorMsg
          });
          currentAssistantMessageId.value = null;
        }
        setStreaming(false);
      }
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
    // ACP
    acpInitialized,
    acpSessionId,
    authMethods,
    initializeAcp,
    authenticateAcp,
    sendAcpMessage,
    cancelAcp,
    stopAcp,
  };
});
