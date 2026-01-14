import { defineStore } from 'pinia';
import { ref, computed, watch } from 'vue';
import type {
  ChatSession,
  ChatMessage,
  ChatConfig,
  ChatProvider,
  ToolCall,
  SendMessageOptions,
} from '../types';
import type { AuthMethod, AgentCapabilities, AcpSessionSummary } from '../services/acpService';
import { acpService } from '../services/acpService';
import { openaiService, type OpenAiConfig } from '../services/openaiService';
import { mcpService } from '../services/mcpService';
import type { TargetInfo } from '../../../shared/types';
import { i18n } from '../../../i18n';
import { appendReasoningBlock, concatAcpTextChunk } from './acpTimeline';
import { createSaveScheduler, loadChatSnapshot, saveChatSnapshot } from './chatPersistence';
import { normalizeSendOptions } from '../pipeline/chatPipeline';
import { createAcpProvider } from '../providers/acpProvider';
import { createOpenaiProvider } from '../providers/openaiProvider';

const t = i18n.global.t;
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
    greeting: t('chat.greeting'),
  });
  const storage = typeof window !== 'undefined' ? window.localStorage : null;
  const saveScheduler = storage
    ? createSaveScheduler(() => {
        saveChatSnapshot(storage, {
          sessions: sessions.value,
          activeSessionId: activeSessionId.value,
        });
      })
    : null;

  // Provider state
  const provider = ref<ChatProvider>('openai');
  const providerInitialized = ref(false);

  // ACP state
  const acpInitialized = ref(false);
  const authMethods = ref<AuthMethod[]>([]);
  const acpCapabilities = ref<AgentCapabilities | null>(null);
  const acpCwd = ref<string | null>(null);
  const acpLoadedSessionId = ref<string | null>(null);
  const acpHistorySummaries = ref<AcpSessionSummary[]>([]);
  const acpHistoryLoading = ref(false);
  const currentAssistantMessageId = ref<string | null>(null);
  const currentAssistantStreamProvider = ref<ChatProvider | null>(null);

  const acpSupportsImages = computed(() => {
    const promptCaps = acpCapabilities.value?.promptCapabilities as
      | { image?: boolean }
      | null
      | undefined;
    if (!promptCaps || typeof promptCaps !== 'object') {
      return false;
    }
    return Boolean(promptCaps.image);
  });

  const providerSupportsImages = computed(() => {
    if (provider.value === 'acp') {
      return acpSupportsImages.value;
    }
    return true;
  });

  let pendingAssistantContent = '';
  let pendingAssistantReasoning = '';
  let flushTimer: ReturnType<typeof setTimeout> | null = null;
  const flushIntervalMs = 50;

  const flushPending = () => {
    if (!currentAssistantMessageId.value || !activeSession.value) {
      pendingAssistantContent = '';
      pendingAssistantReasoning = '';
      currentAssistantStreamProvider.value = null;
      if (flushTimer) {
        clearTimeout(flushTimer);
        flushTimer = null;
      }
      return;
    }

    const messageId = currentAssistantMessageId.value;
    const msg = activeSession.value.messages.find((m) => m.id === messageId);

    if (msg && pendingAssistantReasoning) {
      const delta = pendingAssistantReasoning;
      pendingAssistantReasoning = '';
      const isAcpStream = currentAssistantStreamProvider.value === 'acp';
      const { blocks: nextBlocks, startedNewBlock } = isAcpStream
        ? appendReasoningBlock(msg.blocks, delta, generateId)
        : { blocks: msg.blocks, startedNewBlock: false };
      const existingReasoning = msg.reasoning || '';
      const needsSeparator = startedNewBlock && existingReasoning && !/^\s/.test(delta);
      const deltaForReasoning = needsSeparator ? `\n${delta}` : delta;
      updateMessage(messageId, {
        reasoning: concatAcpTextChunk(existingReasoning, deltaForReasoning),
        blocks: isAcpStream ? nextBlocks : msg.blocks,
      });
    } else {
      pendingAssistantReasoning = '';
    }

    if (pendingAssistantContent) {
      const delta = pendingAssistantContent;
      pendingAssistantContent = '';
      appendToMessage(messageId, delta);
    }

    if (flushTimer) {
      clearTimeout(flushTimer);
      flushTimer = null;
    }
  };

  const scheduleFlush = () => {
    if (flushTimer) {
      return;
    }
    flushTimer = setTimeout(() => {
      flushPending();
    }, flushIntervalMs);
  };

  function queueAssistantContent(delta: string) {
    if (!delta) {
      return;
    }
    pendingAssistantContent += delta;
    scheduleFlush();
  }

  function queueAssistantReasoning(delta: string, merge = false) {
    if (!delta) {
      return;
    }
    pendingAssistantReasoning = merge
      ? concatAcpTextChunk(pendingAssistantReasoning, delta)
      : pendingAssistantReasoning + delta;
    scheduleFlush();
  }

  // OpenAI state
  const openaiInitialized = ref(false);

  function isToolCallCancelled(messageId: string, toolCallId: string): boolean {
    const session = activeSession.value;
    const msg = session?.messages.find((m) => m.id === messageId);
    const tc = msg?.toolCalls?.find((t) => t.id === toolCallId);
    return tc?.status === 'cancelled';
  }

  // Getters
  const activeSession = computed(() =>
    sessions.value.find((s) => s.id === activeSessionId.value) ?? null
  );

  const messages = computed(() => activeSession.value?.messages ?? []);
  const planEntries = computed(() => activeSession.value?.plan ?? []);

  const lastMessage = computed(() => messages.value.at(-1) ?? null);

  const canSend = computed(() => !isStreaming.value && isConnected.value);

  // Actions
  function createSession(title?: string): ChatSession {
    const session: ChatSession = {
      id: generateId(),
      provider: provider.value,
      title: title ?? t('chat.session.title', { index: sessions.value.length + 1 }),
      createdAt: Date.now(),
      updatedAt: Date.now(),
      messages: [],
      totalTokens: 0,
      status: 'idle',
    };
    sessions.value.push(session);
    activeSessionId.value = session.id;
    scheduleSaveToStorage();
    return session;
  }

  function setActiveSession(sessionId: string | null) {
    activeSessionId.value = sessionId;
    scheduleSaveToStorage();
  }

  function deleteSession(sessionId: string) {
    const index = sessions.value.findIndex((s) => s.id === sessionId);
    if (index !== -1) {
      sessions.value.splice(index, 1);
      if (activeSessionId.value === sessionId) {
        activeSessionId.value = sessions.value[0]?.id ?? null;
      }
      scheduleSaveToStorage();
    }
  }

  function clearAllSessions() {
    sessions.value = [];
    activeSessionId.value = null;
    scheduleSaveToStorage();
  }

  function deleteSessionForProvider(sessionId: string, targetProvider: ChatProvider) {
    const session = sessions.value.find((s) => s.id === sessionId);
    if (!session || session.provider !== targetProvider) {
      return;
    }
    deleteSession(sessionId);
  }

  function clearSessionsForProvider(targetProvider: ChatProvider) {
    const remaining = sessions.value.filter((s) => s.provider !== targetProvider);
    sessions.value = remaining;
    if (activeSession.value && activeSession.value.provider === targetProvider) {
      activeSessionId.value = remaining[0]?.id ?? null;
    }
    if (targetProvider === 'openai') {
      void openaiProvider.clearOpenaiContext();
    }
    scheduleSaveToStorage();
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
    scheduleSaveToStorage();
    return newMessage;
  }

  function updateMessage(messageId: string, updates: Partial<ChatMessage>) {
    if (!activeSession.value) return;
    const message = activeSession.value.messages.find((m) => m.id === messageId);
    if (message) {
      Object.assign(message, updates);
      activeSession.value.updatedAt = Date.now();
      scheduleSaveToStorage();
    }
  }

  function appendToMessage(messageId: string, content: string) {
    if (!activeSession.value) return;
    const message = activeSession.value.messages.find((m) => m.id === messageId);
    if (message) {
      message.content += content;
      message.partial = true;
      activeSession.value.updatedAt = Date.now();
      scheduleSaveToStorage();
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
      activeSession.value.plan = undefined;
      activeSession.value.updatedAt = Date.now();
      scheduleSaveToStorage();
    }

    if (provider.value === 'openai') {
      void openaiProvider.clearOpenaiContext();
    }
  }

  function setConfig(newConfig: Partial<ChatConfig>) {
    config.value = { ...config.value, ...newConfig };
  }

  // Persistence
  function loadFromStorage() {
    if (!storage) {
      return;
    }
    const snapshot = loadChatSnapshot(storage);
    if (!snapshot) {
      return;
    }
    sessions.value = snapshot.sessions;
    activeSessionId.value = snapshot.activeSessionId;
  }

  function saveToStorage() {
    if (!storage) {
      return;
    }
    saveChatSnapshot(storage, {
      sessions: sessions.value,
      activeSessionId: activeSessionId.value,
    });
  }

  function scheduleSaveToStorage() {
    if (!saveScheduler) {
      return;
    }
    saveScheduler.schedule();
  }

  function findToolCall(messageId: string, toolCallId: string): ToolCall | undefined {
    const session = activeSession.value;
    if (!session) return undefined;
    const msg = session.messages.find((m) => m.id === messageId);
    return msg?.toolCalls?.find((t) => t.id === toolCallId);
  }

  function addToolCall(messageId: string, toolCall: ToolCall) {
    const session = activeSession.value;
    if (!session) return;
    const msg = session.messages.find((m) => m.id === messageId);
    if (!msg) {
      return;
    }
    if (!msg.toolCalls) {
      msg.toolCalls = [];
    }
    msg.toolCalls.push(toolCall);
  }

  function updateToolCall(messageId: string, toolCallId: string, updates: Partial<ToolCall>) {
    const session = activeSession.value;
    if (!session) return;
    const msg = session.messages.find((m) => m.id === messageId);
    if (!msg?.toolCalls) {
      return;
    }
    const tc = msg.toolCalls.find((t) => t.id === toolCallId);
    if (!tc) {
      return;
    }
    if (updates.result !== undefined) {
      tc.result = (tc.result || '') + updates.result;
    }
    if (updates.status !== undefined) {
      tc.status = updates.status;
    }
    if (updates.name !== undefined) {
      tc.name = updates.name;
    }
    if (updates.arguments !== undefined) {
      tc.arguments = updates.arguments;
    }
  }

  const openaiProvider = createOpenaiProvider(
    {
      activeSession,
      activeSessionId,
      provider,
      providerInitialized,
      openaiInitialized,
      isStreaming,
      currentAssistantMessageId,
      currentAssistantStreamProvider,
      setConnected,
      setStreaming,
      setError,
      addMessage,
      updateMessage,
      scheduleSaveToStorage,
      addToolCall,
      updateToolCall,
      isToolCallCancelled,
      flushPending,
      queueAssistantContent,
      queueAssistantReasoning,
    },
    {
      openaiService,
      mcpService,
      t,
    }
  );

  const acpProvider = createAcpProvider(
    {
      sessions,
      activeSessionId,
      activeSession,
      provider,
      providerInitialized,
      acpInitialized,
      authMethods,
      acpCapabilities,
      acpCwd,
      acpLoadedSessionId,
      acpHistorySummaries,
      acpHistoryLoading,
      currentAssistantMessageId,
      currentAssistantStreamProvider,
      isStreaming,
      setConnected,
      setStreaming,
      setError,
      createSession,
      addMessage,
      updateMessage,
      scheduleSaveToStorage,
      saveToStorage,
      addToolCall,
      updateToolCall,
      findToolCall,
      flushPending,
      queueAssistantContent,
      queueAssistantReasoning,
      generateId,
    },
    {
      acpService,
      t,
    }
  );

  async function initializeAcp(cwd: string, acpArgs?: string, mcpConfigJson?: string) {
    return acpProvider.initializeAcp(cwd, acpArgs, mcpConfigJson);
  }

  async function authenticateAcp(methodId: string) {
    return acpProvider.authenticateAcp(methodId);
  }

  async function refreshAcpHistorySummaries() {
    return acpProvider.refreshAcpHistorySummaries();
  }

  function activateAcpHistorySession(sessionId: string, summary?: AcpSessionSummary) {
    acpProvider.activateAcpHistorySession(sessionId, summary);
  }

  async function deleteAcpHistorySession(sessionId: string) {
    return acpProvider.deleteAcpHistorySession(sessionId);
  }

  async function clearAcpHistorySessions() {
    return acpProvider.clearAcpHistorySessions();
  }

  async function sendAcpMessage(options: SendMessageOptions, cwd?: string) {
    return acpProvider.sendAcpMessage(options, cwd);
  }

  async function cancelAcp() {
    return acpProvider.cancelAcp();
  }

  async function stopAcp() {
    return acpProvider.stopAcp();
  }

  async function initializeOpenai(config: OpenAiConfig, mcpConfigJson = '') {
    openaiProvider.setMcpConfig(mcpConfigJson);
    return openaiProvider.initializeOpenai(config);
  }

  async function refreshOpenaiTools(targets: TargetInfo[]) {
    return openaiProvider.refreshOpenaiTools(targets);
  }

  function updateMcpConfig(configJson: string) {
    openaiProvider.setMcpConfig(configJson);
  }

  async function sendOpenaiMessage(options: SendMessageOptions) {
    return openaiProvider.sendOpenaiMessage(options);
  }

  async function cancelOpenai() {
    return openaiProvider.cancelOpenai();
  }

  async function stopOpenai() {
    return openaiProvider.stopOpenai();
  }

  async function sendMessage(input: string | SendMessageOptions, cwd?: string) {
    const options = normalizeSendOptions(input);
    if (provider.value === 'openai') {
      return sendOpenaiMessage(options);
    }
    return sendAcpMessage(options, cwd);
  }

  watch(
    () => activeSessionId.value,
    () => {
      void acpProvider.maybeLoadAcpHistoryForActiveSession();
    },
    { flush: 'post' }
  );

  watch(
    () => activeSessionId.value,
    () => {
      scheduleSaveToStorage();
      if (provider.value === 'openai') {
        openaiProvider.resetContextForSessionChange();
      }
    },
    { flush: 'post' }
  );

  if (typeof window !== 'undefined') {
    loadFromStorage();
  }
  if (!activeSessionId.value && sessions.value.length > 0) {
    activeSessionId.value = sessions.value[0].id;
  }
  if (sessions.value.length === 0) {
    createSession(t('chat.session.title', { index: 1 }));
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
    planEntries,
    lastMessage,
    canSend,
    // Actions
    createSession,
    setActiveSession,
    deleteSession,
    clearAllSessions,
    deleteSessionForProvider,
    clearSessionsForProvider,
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
    // Provider
    provider,
    providerInitialized,
    providerSupportsImages,
    sendMessage,
    // ACP
    acpInitialized,
    authMethods,
    acpHistorySummaries,
    acpHistoryLoading,
    initializeAcp,
    authenticateAcp,
    refreshAcpHistorySummaries,
    activateAcpHistorySession,
    deleteAcpHistorySession,
    clearAcpHistorySessions,
    sendAcpMessage,
    cancelAcp,
    stopAcp,
    // OpenAI
    openaiInitialized,
    initializeOpenai,
    refreshOpenaiTools,
    updateMcpConfig,
    sendOpenaiMessage,
    cancelOpenai,
    stopOpenai,
  };
});
