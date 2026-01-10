import { defineStore } from 'pinia';
import { ref, computed, watch } from 'vue';
import type {
  ChatSession,
  ChatMessage,
  ChatConfig,
  ToolCall,
  PlanEntry,
  PlanEntryPriority,
  PlanEntryStatus,
  ImageAttachment,
  TextAttachment,
  PromptBlock,
  SendMessageOptions,
} from '../types';
import type {
  AuthMethod,
  AcpEvent,
  AgentCapabilities,
  AcpContentBlock,
  AcpSessionSummary,
} from '../services/acpService';
import { acpService } from '../services/acpService';
import { openaiService, type OpenAiConfig, type ChatStreamEvent, type OpenAiContentPart } from '../services/openaiService';
import { fetchTargets } from '../../../services/api';
import type { TargetInfo } from '../../../shared/types';
import { i18n } from '../../../i18n';
import { appendReasoningBlock, concatAcpTextChunk, ensureToolCallBlock } from './acpTimeline';
import { createSaveScheduler, loadChatSnapshot, saveChatSnapshot } from './chatPersistence';

const t = i18n.global.t;
const TOOL_CALL_CONCURRENCY_LIMIT = 10;

export type ChatProvider = 'acp' | 'openai';

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
  let currentAssistantStreamProvider: ChatProvider | null = null;
  let acpEventUnlisten: (() => void) | null = null;

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
      currentAssistantStreamProvider = null;
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
      const isAcpStream = currentAssistantStreamProvider === 'acp';
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

  // OpenAI state
  const openaiInitialized = ref(false);
  let openaiEventUnlisten: (() => void) | null = null;
  let openaiListenerToken = 0;
  const openaiContextSessionId = ref<string | null>(null);
  let openaiContextQueue: Promise<void> = Promise.resolve();
  let openaiToolAbortController: AbortController | null = null;
  const openaiToolsSignature = ref('');

  function enqueueOpenaiContextOp(op: () => Promise<void>) {
    openaiContextQueue = openaiContextQueue
      .then(op)
      .catch((e) => {
        console.warn('[chatStore] openai context op failed:', e);
      });
    return openaiContextQueue;
  }

  function beginOpenaiToolRun(): AbortSignal {
    if (openaiToolAbortController) {
      openaiToolAbortController.abort();
    }
    openaiToolAbortController = new AbortController();
    return openaiToolAbortController.signal;
  }

  function abortOpenaiToolRun() {
    if (openaiToolAbortController) {
      openaiToolAbortController.abort();
    }
  }

  function toolCancelPromise(signal: AbortSignal): Promise<'cancelled'> {
    if (signal.aborted) {
      return Promise.resolve('cancelled');
    }
    return new Promise((resolve) => {
      const onAbort = () => {
        signal.removeEventListener('abort', onAbort);
        resolve('cancelled');
      };
      signal.addEventListener('abort', onAbort, { once: true });
    });
  }

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

  function toPlainText(value: unknown): string {
    if (value === null || value === undefined) return '';
    if (typeof value === 'string') return value;
    if (typeof value === 'number' || typeof value === 'boolean') return String(value);
    if (Array.isArray(value)) return value.map(toPlainText).filter(Boolean).join('');
    if (typeof value === 'object') {
      const obj = value as Record<string, unknown>;
      if (typeof obj.text === 'string') return obj.text;
      if (typeof obj.content === 'string') return obj.content;
      if (typeof obj.message === 'string') return obj.message;
      if (typeof obj.value === 'string') return obj.value;
      if (Array.isArray(obj.content)) return toPlainText(obj.content);
      if (Array.isArray(obj.prompt)) return toPlainText(obj.prompt);
      if (Array.isArray(obj.messages)) return toPlainText(obj.messages);
      if (Array.isArray(obj.blocks)) return toPlainText(obj.blocks);
    }
    return '';
  }

  function toRole(value: unknown): 'user' | 'assistant' | 'system' {
    if (!value || typeof value !== 'string') return 'assistant';
    const v = value.toLowerCase();
    if (v === 'user') return 'user';
    if (v === 'assistant') return 'assistant';
    if (v === 'system') return 'system';
    return 'assistant';
  }

  function toTimestamp(value: unknown): number | null {
    if (typeof value === 'number' && Number.isFinite(value)) return value;
    return null;
  }

  function parseAcpHistory(history: unknown): ChatMessage[] {
    if (!history) return [];

    const list = Array.isArray(history)
      ? history
      : typeof history === 'object' && history !== null && Array.isArray((history as any).items)
        ? ((history as any).items as unknown[])
        : typeof history === 'object' && history !== null && Array.isArray((history as any).history)
          ? ((history as any).history as unknown[])
          : [];

    if (!Array.isArray(list) || list.length === 0) return [];

    const now = Date.now();
    const parsed: ChatMessage[] = [];

    for (let i = 0; i < list.length; i += 1) {
      const item = list[i];
      const obj = typeof item === 'object' && item !== null ? (item as Record<string, unknown>) : null;
      const role = toRole(obj?.role ?? obj?.speaker ?? obj?.type);

      const content = toPlainText(
        obj?.content ??
          obj?.text ??
          obj?.message ??
          obj?.value ??
          obj?.prompt ??
          obj?.output ??
          obj?.response
      );

      if (!content.trim()) {
        continue;
      }

      const ts =
        toTimestamp(obj?.ts) ??
        toTimestamp(obj?.timestamp) ??
        toTimestamp(obj?.createdAt) ??
        toTimestamp(obj?.created_at) ??
        (now - (list.length - i) * 1000);

      parsed.push({
        id: generateId(),
        ts,
        type: 'say',
        say: 'text',
        role,
        content,
        status: 'complete',
        partial: false,
      });
    }

    return parsed;
  }

  function applyAcpHistoryToActiveSession(history: unknown) {
    const session = activeSession.value;
    if (!session) return;

    const parsed = parseAcpHistory(history);
    if (parsed.length === 0) return;

    const current = session.messages || [];
    const currentSignature = current
      .slice(-3)
      .map((m) => `${m.role}:${(m.content || '').trim()}`)
      .join('|');
    const parsedSignature = parsed
      .slice(-3)
      .map((m) => `${m.role}:${(m.content || '').trim()}`)
      .join('|');

    if (current.length === 0 || currentSignature !== parsedSignature || current.length < parsed.length) {
      session.messages = parsed;
      session.messageCount = parsed.length;
      session.updatedAt = Date.now();
      saveToStorage();
    }
  }

  let acpHistoryLoadToken = 0;

  async function loadAcpSessionOrThrow(sessionId: string) {
    try {
      const loaded = await acpService.loadSession(sessionId);
      acpLoadedSessionId.value = sessionId;
      return loaded;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(`ACP load session failed: ${msg}`);
      throw e;
    }
  }

  async function maybeLoadAcpHistoryForActiveSession() {
    if (!acpInitialized.value || provider.value !== 'acp') {
      return;
    }
    if (!canLoadAcpSession()) {
      return;
    }
    if (isStreaming.value) {
      return;
    }
    const session = activeSession.value;
    if (!session?.acpSessionId) {
      return;
    }
    if (acpLoadedSessionId.value === session.acpSessionId) {
      return;
    }

    const token = ++acpHistoryLoadToken;
    try {
      const loaded = await loadAcpSessionOrThrow(session.acpSessionId);
      if (token !== acpHistoryLoadToken) {
        return;
      }
      applyAcpHistoryToActiveSession(loaded.history);
    } catch {
      // error already set via setError
    }
  }

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
      openaiContextSessionId.value = null;
      if (openaiInitialized.value && provider.value === 'openai') {
        void enqueueOpenaiContextOp(async () => {
          await openaiService.clearMessages();
        });
      }
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
      openaiContextSessionId.value = null;
      if (openaiInitialized.value) {
        void enqueueOpenaiContextOp(async () => {
          await openaiService.clearMessages();
        });
      }
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

  async function refreshAcpHistorySummaries() {
    if (!acpInitialized.value) {
      acpHistorySummaries.value = [];
      return;
    }
    if (acpHistoryLoading.value) {
      return;
    }
    acpHistoryLoading.value = true;
    try {
      const result = await acpService.listSessions();
      acpHistorySummaries.value = result.sessions ?? [];
    } catch (e) {
      console.warn('[chatStore] load ACP history failed:', e);
      acpHistorySummaries.value = [];
    } finally {
      acpHistoryLoading.value = false;
    }
  }

  function findAcpSessionByRemoteId(sessionId: string) {
    return sessions.value.find((s) => s.provider === 'acp' && s.acpSessionId === sessionId);
  }

  function activateAcpHistorySession(sessionId: string, summary?: AcpSessionSummary) {
    const existing = findAcpSessionByRemoteId(sessionId);
    if (existing) {
      if (summary?.title && summary.title !== existing.title) {
        existing.title = summary.title;
      }
      if (typeof summary?.messageCount === 'number') {
        existing.messageCount = summary.messageCount;
      }
      if (typeof summary?.updatedAt === 'number') {
        existing.updatedAt = Math.max(existing.updatedAt, summary.updatedAt);
      }
      activeSessionId.value = existing.id;
      scheduleSaveToStorage();
      return;
    }

    const createdAt = summary?.createdAt ?? Date.now();
    const updatedAt = summary?.updatedAt ?? createdAt;
    const title = summary?.title ?? t('chat.session.title', { index: sessions.value.length + 1 });
    const session: ChatSession = {
      id: sessionId,
      provider: 'acp',
      title,
      createdAt,
      updatedAt,
      messages: [],
      messageCount: summary?.messageCount,
      totalTokens: 0,
      status: 'idle',
      acpSessionId: sessionId,
    };
    sessions.value.push(session);
    activeSessionId.value = session.id;
    scheduleSaveToStorage();
  }

  async function deleteAcpHistorySession(sessionId: string) {
    const active = activeSession.value;
    const deletingActive = active?.provider === 'acp' && active.acpSessionId === sessionId;

    if (deletingActive) {
      await cancelAcp();
      currentAssistantMessageId.value = null;
      currentAssistantStreamProvider = null;
      pendingAssistantContent = '';
      pendingAssistantReasoning = '';
      acpLoadedSessionId.value = null;
    } else if (acpLoadedSessionId.value === sessionId) {
      acpLoadedSessionId.value = null;
    }

    await acpService.deleteSession(sessionId);
    acpHistorySummaries.value = acpHistorySummaries.value.filter(
      (item) => item.sessionId !== sessionId
    );
    const activeId = activeSessionId.value;
    sessions.value = sessions.value.filter(
      (session) => !(session.provider === 'acp' && session.acpSessionId === sessionId)
    );
    if (deletingActive) {
      activeSessionId.value = null;
      createSession();
    } else if (activeId && !sessions.value.find((session) => session.id === activeId)) {
      activeSessionId.value = sessions.value[0]?.id ?? null;
      if (!activeSessionId.value && provider.value === 'acp') {
        createSession();
      }
    }
    scheduleSaveToStorage();
  }

  async function clearAcpHistorySessions() {
    const active = activeSession.value;
    if (active?.provider === 'acp') {
      await cancelAcp();
      currentAssistantMessageId.value = null;
      currentAssistantStreamProvider = null;
      pendingAssistantContent = '';
      pendingAssistantReasoning = '';
      acpLoadedSessionId.value = null;
    }

    const summaries = [...acpHistorySummaries.value];
    for (const item of summaries) {
      await acpService.deleteSession(item.sessionId);
    }
    acpHistorySummaries.value = [];
    sessions.value = sessions.value.filter((session) => session.provider !== 'acp');
    if (provider.value === 'acp') {
      activeSessionId.value = null;
      createSession();
    } else if (activeSessionId.value && !sessions.value.find((s) => s.id === activeSessionId.value)) {
      activeSessionId.value = sessions.value[0]?.id ?? null;
    }
    scheduleSaveToStorage();
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

  function isFinalToolCallStatus(status: ToolCall['status']) {
    return status === 'completed' || status === 'failed' || status === 'cancelled';
  }

  function closePendingOpenaiToolCalls(reason: string): boolean {
    const session = activeSession.value;
    if (!session) {
      return false;
    }
    let changed = false;
    for (const msg of session.messages) {
      if (msg.role !== 'assistant' || !Array.isArray(msg.toolCalls)) {
        continue;
      }
      for (const tc of msg.toolCalls) {
        if (tc.status !== 'pending' && tc.status !== 'running') {
          if (isFinalToolCallStatus(tc.status)) {
            const result = (tc.result || '').trim();
            if (!result) {
              tc.result = t('chat.tool.result.missing', { status: tc.status });
              changed = true;
            }
          }
          continue;
        }
        const note = t('chat.tool.cancelled', { reason });
        if (tc.result && tc.result.trim()) {
          tc.result = `${tc.result}\n${note}`;
        } else {
          tc.result = note;
        }
        tc.status = 'cancelled';
        changed = true;
      }
    }
    if (changed) {
      scheduleSaveToStorage();
    }
    return changed;
  }

  async function syncOpenaiContextForSession(session: ChatSession | null) {
    if (!openaiInitialized.value || provider.value !== 'openai') {
      return;
    }
    if (isStreaming.value) {
      return;
    }
    await enqueueOpenaiContextOp(async () => {
      await openaiService.clearMessages();
      const list = session?.messages ?? [];
      for (const msg of list) {
        if (msg.status === 'streaming') {
          continue;
        }

        if (msg.role === 'user' || msg.role === 'assistant' || msg.role === 'system') {
          const content = (msg.content || '').trim();
          if (msg.role === 'assistant' && Array.isArray(msg.toolCalls) && msg.toolCalls.length > 0) {
            const tool_calls = msg.toolCalls.map((tc) => {
              const args = tc.arguments ? JSON.stringify(tc.arguments) : '{}';
              return {
                id: tc.id,
                type: 'function',
                function: {
                  name: tc.name,
                  arguments: args,
                },
              };
            });
            await openaiService.addMessage({ role: 'assistant', content, tool_calls });
          } else if (msg.role === 'user') {
            const parts: OpenAiContentPart[] = [];
            if (content) {
              parts.push({ type: 'text', text: content });
            }
            if (Array.isArray(msg.images)) {
              for (const url of msg.images) {
                if (typeof url === 'string' && url.trim()) {
                  parts.push({ type: 'image_url', image_url: { url } });
                }
              }
            }
            if (parts.length > 0) {
              await openaiService.addMessage({ role: 'user', content: parts });
            }
          } else if (content) {
            await openaiService.addMessage({ role: msg.role, content });
          }

          if (Array.isArray(msg.toolCalls)) {
            for (const tc of msg.toolCalls) {
              if (!isFinalToolCallStatus(tc.status)) {
                continue;
              }
              const result = (tc.result || '').trim();
              if (!result) {
                continue;
              }
              await openaiService.addMessage({
                role: 'tool',
                content: result,
                tool_call_id: tc.id,
              });
            }
          }
        }
      }
    });
    openaiContextSessionId.value = session?.id ?? null;
  }

  async function ensureOpenaiContextForActiveSession() {
    const session = activeSession.value;
    if (!session) {
      return;
    }
    if (openaiContextSessionId.value === session.id) {
      return;
    }
    await syncOpenaiContextForSession(session);
  }

  // ACP Actions
  async function initializeAcp(cwd: string, acpArgs?: string) {
    console.log('[chatStore] initializeAcp called with cwd:', cwd);
    try {
      console.log('[chatStore] initializeAcp: calling acpService.start...');
      const response = await acpService.start(cwd, acpArgs);
      console.log('[chatStore] initializeAcp: acpService.start returned:', response);
      authMethods.value = response.authMethods;
      acpInitialized.value = true;
      setConnected(true);
      acpCapabilities.value = response.agentCapabilities ?? null;
      provider.value = 'acp';
      providerInitialized.value = true;
      acpCwd.value = cwd;
      acpLoadedSessionId.value = null;
      openaiContextSessionId.value = null;
      setupAcpEventListener();
      if (activeSession.value?.acpSessionId) {
        try {
          if (canLoadAcpSession()) {
            const loaded = await loadAcpSessionOrThrow(activeSession.value.acpSessionId);
            applyAcpHistoryToActiveSession(loaded.history);
          }
        } catch {
          // error already set via setError
        }
      }
      console.log('[chatStore] initializeAcp done');
      return response;
    } catch (e) {
      console.error('[chatStore] initializeAcp failed:', e);
      setError(`Failed to initialize ACP: ${e}`);
      setConnected(false);
      throw e;
    }
  }

  async function authenticateAcp(methodId: string) {
    try {
      await acpService.authenticate(methodId);
      setConnected(true);
    } catch (e) {
      setConnected(false);
      setError(`Authentication failed: ${e}`);
      throw e;
    }
  }

  async function ensureAcpSessionLoaded(cwd: string): Promise<string> {
    if (!activeSession.value) {
      createSession();
    }
    const session = activeSession.value!;

    if (session.acpSessionId) {
      if (canLoadAcpSession() && acpLoadedSessionId.value !== session.acpSessionId) {
        const loaded = await loadAcpSessionOrThrow(session.acpSessionId);
        applyAcpHistoryToActiveSession(loaded.history);
      }
      return session.acpSessionId;
    }

    const info = await acpService.newSession(cwd);
    session.acpSessionId = info.sessionId;
    acpLoadedSessionId.value = info.sessionId;
    session.updatedAt = Date.now();
    saveToStorage();
    return info.sessionId;
  }

  watch(
    () => activeSessionId.value,
    () => {
      void maybeLoadAcpHistoryForActiveSession();
    },
    { flush: 'post' }
  );

  async function sendAcpMessage(options: SendMessageOptions, cwd?: string) {
    if (!acpInitialized.value) {
      throw new Error('ACP not initialized');
    }

    const resolvedCwd = cwd ?? acpCwd.value ?? '.';
    await ensureAcpSessionLoaded(resolvedCwd);

    const promptBlocks = buildPromptBlocks(options);
    const acpPromptBlocks = toAcpPromptBlocks(promptBlocks);
    if (acpPromptBlocks.length === 0) {
      return;
    }

    const content = options.content ?? '';
    // Add user message
    addMessage({
      type: 'say',
      say: 'text',
      role: 'user',
      content,
      status: 'complete',
      images: toDisplayImages(options.images),
      files: toDisplayFiles(options.files),
    });

    // Add assistant placeholder
    const assistantMsg = addMessage({
      type: 'say',
      say: 'text',
      role: 'assistant',
      content: '',
      status: 'streaming',
      partial: true,
      blocks: [],
    });
    currentAssistantMessageId.value = assistantMsg.id;
    currentAssistantStreamProvider = 'acp';

    setStreaming(true);

    try {
      await acpService.prompt(acpPromptBlocks, options.context);
    } catch (e) {
      updateMessage(assistantMsg.id, { status: 'error', content: `Error: ${e}` });
      setStreaming(false);
      currentAssistantMessageId.value = null;
      currentAssistantStreamProvider = null;
      throw e;
    }
  }

  async function cancelAcp() {
    try {
      flushPending();
      await acpService.cancel();
      setStreaming(false);
      if (currentAssistantMessageId.value) {
        const msg = activeSession.value?.messages.find(m => m.id === currentAssistantMessageId.value);
        if (msg && (!msg.content || !msg.content.trim())) {
          updateMessage(currentAssistantMessageId.value, { status: 'cancelled', content: t('chat.response.stopped'), partial: false });
        } else {
          updateMessage(currentAssistantMessageId.value, { status: 'cancelled', partial: false });
        }
        currentAssistantMessageId.value = null;
        currentAssistantStreamProvider = null;
      }
    } catch (e) {
      console.error('Failed to cancel:', e);
    }
  }

  async function stopAcp() {
    console.log('[chatStore] stopAcp called, acpInitialized:', acpInitialized.value);
    if (acpEventUnlisten) {
      console.log('[chatStore] stopAcp: unlisten event');
      acpEventUnlisten();
      acpEventUnlisten = null;
    }
    // Only call backend stop if ACP was actually initialized
    if (acpInitialized.value) {
      try {
        console.log('[chatStore] stopAcp: calling acpService.stop()');
        await acpService.stop();
        console.log('[chatStore] stopAcp: acpService.stop() done');
      } catch (e) {
        console.error('Failed to stop ACP:', e);
      }
    }
    acpInitialized.value = false;
    acpCapabilities.value = null;
    acpCwd.value = null;
    acpLoadedSessionId.value = null;
    providerInitialized.value = false;
    setConnected(false);
    console.log('[chatStore] stopAcp done');
  }

  function canLoadAcpSession() {
    return acpCapabilities.value?.loadSession !== false;
  }

  function setupAcpEventListener() {
    if (acpEventUnlisten) {
      acpEventUnlisten();
      acpEventUnlisten = null;
    }
    acpService.onEvent(handleAcpEvent).then((unlisten) => {
      acpEventUnlisten = unlisten;
    });
  }

  function findToolCall(messageId: string, toolCallId: string): ToolCall | undefined {
    const session = activeSession.value;
    if (!session) return undefined;
    const msg = session.messages.find((m) => m.id === messageId);
    return msg?.toolCalls?.find((t) => t.id === toolCallId);
  }

  function mapAcpToolStatus(status: unknown): ToolCall['status'] | undefined {
    if (typeof status !== 'string') {
      return undefined;
    }
    switch (status) {
      case 'pending':
        return 'pending';
      case 'in_progress':
        return 'running';
      case 'completed':
        return 'completed';
      case 'failed':
        return 'failed';
      default:
        return undefined;
    }
  }

  function mapPlanStatus(status: unknown): PlanEntryStatus {
    if (typeof status !== 'string') {
      return 'pending';
    }
    switch (status) {
      case 'pending':
        return 'pending';
      case 'in_progress':
        return 'in_progress';
      case 'completed':
        return 'completed';
      default:
        return 'pending';
    }
  }

  function mapPlanPriority(priority: unknown): PlanEntryPriority {
    if (typeof priority !== 'string') {
      return 'medium';
    }
    switch (priority) {
      case 'low':
        return 'low';
      case 'high':
        return 'high';
      case 'medium':
        return 'medium';
      default:
        return 'medium';
    }
  }

  function normalizePlanEntries(value: unknown): PlanEntry[] {
    if (!Array.isArray(value)) {
      return [];
    }
    const entries: PlanEntry[] = [];
    for (const entry of value) {
      if (!entry || typeof entry !== 'object') {
        continue;
      }
      const obj = entry as Record<string, unknown>;
      const rawContent = obj.content ?? obj.step ?? obj.title;
      const content =
        typeof rawContent === 'string' ? rawContent.trim() : stringifyForDisplay(rawContent);
      if (!content || !content.trim()) {
        continue;
      }
      entries.push({
        content: content.trim(),
        status: mapPlanStatus(obj.status),
        priority: mapPlanPriority(obj.priority),
      });
    }
    return entries;
  }

  function normalizeToolArguments(rawInput: unknown): Record<string, unknown> {
    if (rawInput && typeof rawInput === 'object' && !Array.isArray(rawInput)) {
      return rawInput as Record<string, unknown>;
    }
    if (rawInput === undefined) {
      return {};
    }
    return { value: rawInput };
  }

  function stringifyForDisplay(value: unknown): string {
    if (value === undefined) {
      return '';
    }
    if (typeof value === 'string') {
      return value;
    }
    try {
      return JSON.stringify(value, null, 2);
    } catch {
      return String(value);
    }
  }

  function extractContentText(content: unknown): string | null {
    if (!content || typeof content !== 'object') {
      return null;
    }
    const maybeText = (content as { text?: unknown }).text;
    return typeof maybeText === 'string' ? maybeText : null;
  }

  function formatToolCallContent(content: unknown): string | null {
    if (!Array.isArray(content)) {
      return null;
    }
    const parts: string[] = [];
    for (const item of content) {
      if (item && typeof item === 'object') {
        const itemObj = item as Record<string, unknown>;
        if (itemObj.type === 'content') {
          const text = extractContentText(itemObj.content);
          if (text) {
            parts.push(text);
            continue;
          }
        }
      }
      parts.push(stringifyForDisplay(item));
    }
    return parts.length > 0 ? parts.join('\n') : null;
  }

  function extractToolOutput(update: Record<string, unknown>, existing?: ToolCall): string | null {
    const meta = (update._meta ?? update.meta) as Record<string, unknown> | undefined;
    const terminalOutput = meta?.terminal_output as Record<string, unknown> | undefined;
    const terminalData = terminalOutput?.data;
    if (typeof terminalData === 'string') {
      return terminalData;
    }

    const contentText = formatToolCallContent(update.content);
    if (contentText) {
      return contentText;
    }

    const hasExistingOutput = Boolean(existing?.result);
    if (!hasExistingOutput && update.rawOutput !== undefined) {
      return stringifyForDisplay(update.rawOutput);
    }
    if (!hasExistingOutput && update.raw_output !== undefined) {
      return stringifyForDisplay(update.raw_output);
    }

    return null;
  }

  function extractAcpSessionId(value: unknown): string | null {
    if (!value || typeof value !== 'object') {
      return null;
    }
    const payload = value as Record<string, unknown>;
    const direct = payload.sessionId ?? payload.session_id;
    if (typeof direct === 'string') {
      return direct;
    }

    const update = payload.update;
    if (update && typeof update === 'object') {
      const updateObj = update as Record<string, unknown>;
      const updateSessionId = updateObj.sessionId ?? updateObj.session_id;
      if (typeof updateSessionId === 'string') {
        return updateSessionId;
      }
    }

    return null;
  }

  function handleAcpEvent(event: AcpEvent) {
    const method = event.type;
    const payload = event.payload as Record<string, unknown>;

    if (provider.value !== 'acp') {
      return;
    }

    const activeAcpSessionId = activeSession.value?.acpSessionId ?? null;
    const eventSessionId = extractAcpSessionId(payload);
    if (eventSessionId) {
      if (!activeAcpSessionId || eventSessionId !== activeAcpSessionId) {
        return;
      }
    } else if (!activeAcpSessionId && method === 'session/update') {
      return;
    }

    if (method === 'session/update') {
      const update = payload.update as Record<string, unknown> | undefined;
      if (!update) return;

      const sessionUpdate = (update.sessionUpdate ?? update.session_update) as string | undefined;

      if (sessionUpdate === 'agent_message_chunk' || sessionUpdate === 'agent_thought_chunk' || sessionUpdate === 'agent_reasoning_chunk') {
        const contentText = extractContentText(update.content);
        if (contentText && currentAssistantMessageId.value) {
          if (sessionUpdate === 'agent_thought_chunk' || sessionUpdate === 'agent_reasoning_chunk') {
            pendingAssistantReasoning = concatAcpTextChunk(pendingAssistantReasoning, contentText);
          } else {
            pendingAssistantContent += contentText;
          }
          scheduleFlush();
        }
      } else if (sessionUpdate === 'tool_call') {
        flushPending();
        const messageId = currentAssistantMessageId.value;
        if (!messageId) {
          return;
        }
        const toolCallId = (update.toolCallId ?? update.tool_call_id) as string | undefined;
        const title = update.title as string | undefined;
        const name = update.name as string | undefined;
        const rawInput = update.rawInput ?? update.raw_input;
        const status = mapAcpToolStatus(update.status);
        if (toolCallId && !findToolCall(messageId, toolCallId)) {
          addToolCall(messageId, {
            id: toolCallId,
            name: name || title || 'Unknown Tool',
            arguments: normalizeToolArguments(rawInput),
            status: status ?? 'running',
          });
        }
        if (toolCallId) {
          const session = activeSession.value;
          if (!session) {
            return;
          }
          const msg = session.messages.find((m) => m.id === messageId);
          if (msg) {
            const { blocks, inserted } = ensureToolCallBlock(msg.blocks, toolCallId);
            if (inserted) {
              updateMessage(messageId, { blocks });
            } else {
              session.updatedAt = Date.now();
              scheduleSaveToStorage();
            }
          }
        }
      } else if (sessionUpdate === 'tool_call_update') {
        const toolCallId = (update.toolCallId ?? update.tool_call_id) as string | undefined;
        if (currentAssistantMessageId.value && toolCallId) {
          const existing = findToolCall(currentAssistantMessageId.value, toolCallId);
          const title = update.title as string | undefined;
          const name = update.name as string | undefined;
          const rawInput = update.rawInput ?? update.raw_input;
          const status = mapAcpToolStatus(update.status);
          const updates: Partial<ToolCall> = {};
          const output = extractToolOutput(update, existing);
          if (output) {
            updates.result = output;
          }
          if (status) {
            updates.status = status;
          }
          if (title || name) {
            updates.name = name || title;
          }
          if (rawInput !== undefined) {
            updates.arguments = normalizeToolArguments(rawInput);
          }
          if (!existing) {
            flushPending();
            addToolCall(currentAssistantMessageId.value, {
              id: toolCallId,
              name: updates.name || 'Unknown Tool',
              arguments: updates.arguments ?? {},
              status: updates.status ?? 'running',
              result: updates.result,
            });
          } else {
            updateToolCall(currentAssistantMessageId.value, toolCallId, updates);
          }
          const session = activeSession.value;
          if (!session) {
            return;
          }
          const msg = session.messages.find((m) => m.id === currentAssistantMessageId.value);
          if (msg) {
            const { blocks, inserted } = ensureToolCallBlock(msg.blocks, toolCallId);
            if (inserted) {
              updateMessage(currentAssistantMessageId.value, { blocks });
            }
            session.updatedAt = Date.now();
            scheduleSaveToStorage();
          }
        }
      } else if (sessionUpdate === 'retry') {
        const attempt = (update.attempt as number | undefined) ?? 0;
        const maxAttempts =
          (update.maxAttempts as number | undefined) ??
          (update.max_attempts as number | undefined) ??
          0;
        const message = (update.message as string | undefined) ?? 'Retrying...';
        if (currentAssistantMessageId.value) {
          const retryToolCallId = 'acp-retry';
          if (!findToolCall(currentAssistantMessageId.value, retryToolCallId)) {
            flushPending();
            addToolCall(currentAssistantMessageId.value, {
              id: retryToolCallId,
              name: 'Retry',
              arguments: { attempt, maxAttempts },
              status: 'running',
            });
          }
          updateToolCall(currentAssistantMessageId.value, retryToolCallId, {
            status: 'running',
            arguments: { attempt, maxAttempts },
            result: `[${attempt}/${maxAttempts}] ${message}\n`,
          });
          const session = activeSession.value;
          if (!session) {
            return;
          }
          const msg = session.messages.find((m) => m.id === currentAssistantMessageId.value);
          if (msg) {
            const { blocks, inserted } = ensureToolCallBlock(msg.blocks, retryToolCallId);
            if (inserted) {
              updateMessage(currentAssistantMessageId.value, { blocks });
            }
            session.updatedAt = Date.now();
            scheduleSaveToStorage();
          }
        }
      } else if (sessionUpdate === 'plan') {
        const rawEntries =
          (update.entries as unknown) ??
          ((update.plan as Record<string, unknown> | undefined)?.entries as unknown);
        const entries = normalizePlanEntries(rawEntries);
        if (activeSession.value) {
          activeSession.value.plan = entries;
          activeSession.value.updatedAt = Date.now();
          scheduleSaveToStorage();
        }
      } else if (sessionUpdate === 'task_complete') {
        flushPending();
        if (currentAssistantMessageId.value) {
          const retryToolCallId = 'acp-retry';
          const existingRetry = findToolCall(currentAssistantMessageId.value, retryToolCallId);
          if (existingRetry && existingRetry.status === 'running') {
            updateToolCall(currentAssistantMessageId.value, retryToolCallId, { status: 'completed' });
          }
          updateMessage(currentAssistantMessageId.value, { status: 'complete', partial: false });
          currentAssistantMessageId.value = null;
          currentAssistantStreamProvider = null;
        }
        setStreaming(false);
      } else if (sessionUpdate === 'error') {
        flushPending();
        const errorMsg = (update.error as { message?: string })?.message || 'Unknown error';
        setError(errorMsg);
        if (currentAssistantMessageId.value) {
          const retryToolCallId = 'acp-retry';
          const existingRetry = findToolCall(currentAssistantMessageId.value, retryToolCallId);
          if (existingRetry && existingRetry.status === 'running') {
            updateToolCall(currentAssistantMessageId.value, retryToolCallId, { status: 'failed' });
          }
          updateMessage(currentAssistantMessageId.value, { status: 'error', content: errorMsg });
          currentAssistantMessageId.value = null;
          currentAssistantStreamProvider = null;
        }
        setStreaming(false);
      }
    } else if (method === 'prompt/complete') {
      // Handle prompt completion (stopReason: "end_turn")
      console.log('[chatStore] prompt/complete received:', payload);
      flushPending();
      if (currentAssistantMessageId.value) {
        const retryToolCallId = 'acp-retry';
        const existingRetry = findToolCall(currentAssistantMessageId.value, retryToolCallId);
        if (existingRetry && existingRetry.status === 'running') {
          updateToolCall(currentAssistantMessageId.value, retryToolCallId, { status: 'completed' });
        }
        updateMessage(currentAssistantMessageId.value, { status: 'complete', partial: false });
        currentAssistantMessageId.value = null;
        currentAssistantStreamProvider = null;
      }
      setStreaming(false);
    }
  }

  function addToolCall(messageId: string, toolCall: ToolCall) {
    const session = activeSession.value;
    if (!session) return;
    const msg = session.messages.find((m) => m.id === messageId);
    if (msg) {
      if (!msg.toolCalls) {
        msg.toolCalls = [];
      }
      msg.toolCalls.push(toolCall);
    }
  }

  function updateToolCall(messageId: string, toolCallId: string, updates: Partial<ToolCall>) {
    const session = activeSession.value;
    if (!session) return;
    const msg = session.messages.find((m) => m.id === messageId);
    if (msg?.toolCalls) {
      const tc = msg.toolCalls.find((t) => t.id === toolCallId);
      if (tc) {
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
    }
  }

  // OpenAI Actions
  async function initializeOpenai(config: OpenAiConfig) {
    console.log('[chatStore] initializeOpenai called');
    try {
      console.log('[chatStore] initializeOpenai: calling openaiService.init()');
      await openaiService.init(config);
      console.log('[chatStore] initializeOpenai: openaiService.init() done');
      openaiToolsSignature.value = '';

      // Always register base tools so tools are available even before targets load.
      try {
        await openaiService.setTools(buildOpenaiTools([], undefined));
        openaiToolsSignature.value = buildOpenaiToolsSignature([], undefined);
      } catch (e) {
        console.warn('[chatStore] initializeOpenai: setTools base failed (continuing without tools):', e);
      }

      // Provide tools for OpenAI tool calling (targets are sourced from console/proxy)
      try {
        const targets = await fetchTargets();
        const targetNames = buildTargetNameList(targets);
        const defaultTarget = resolveDefaultTarget(targets, targetNames);
        const signature = buildOpenaiToolsSignature(targetNames, defaultTarget);
        if (signature !== openaiToolsSignature.value) {
          await openaiService.setTools(buildOpenaiTools(targetNames, defaultTarget));
          openaiToolsSignature.value = signature;
        }
      } catch (e) {
        console.warn('[chatStore] initializeOpenai: refresh tools from targets failed:', e);
      }

      openaiInitialized.value = true;
      providerInitialized.value = true;
      provider.value = 'openai';
      setupOpenaiEventListener();
      setConnected(true);
      await syncOpenaiContextForSession(activeSession.value);
      console.log('[chatStore] initializeOpenai done');
    } catch (e) {
      console.error('[chatStore] initializeOpenai failed:', e);
      setError(`Failed to initialize OpenAI: ${e}`);
      throw e;
    }
  }

  function buildTargetNameList(targets: TargetInfo[]) {
    return Array.from(new Set(targets.map((t) => t.name))).sort((a, b) => a.localeCompare(b));
  }

  function resolveDefaultTarget(targets: TargetInfo[], targetNames: string[]) {
    return targets.find((t) => t.is_default)?.name ?? targetNames[0];
  }

  function buildOpenaiToolsSignature(targetNames: string[], defaultTarget?: string) {
    return `${targetNames.join(',')}|${defaultTarget ?? ''}`;
  }

  function buildOpenaiTools(targets: string[], defaultTarget?: string) {
    const targetProperty: Record<string, unknown> = {
      type: 'string',
      description: 'Target name defined in octovalve-proxy config.',
    };
    if (targets.length > 0) {
      targetProperty.enum = targets;
    }
    if (defaultTarget) {
      targetProperty.default = defaultTarget;
    }

    return [
      {
        type: 'function',
        function: {
          name: 'run_command',
          description: 'Forward command execution to a remote broker with manual approval. target is required.',
          parameters: {
            type: 'object',
            additionalProperties: false,
            properties: {
              command: {
                type: 'string',
                description: 'Shell-like command line. Default mode executes via /bin/bash -lc.',
              },
              target: targetProperty,
              intent: {
                type: 'string',
                description: 'Why this command is needed (required for audit).',
              },
              mode: {
                type: 'string',
                enum: ['shell', 'argv'],
                default: 'shell',
                description: 'Execution mode: shell uses /bin/bash -lc, argv uses parsed pipeline.',
              },
              cwd: {
                type: 'string',
                description: 'Working directory for the command.',
              },
              timeout_ms: {
                type: 'integer',
                minimum: 0,
                description: 'Override command timeout in milliseconds.',
              },
              max_output_bytes: {
                type: 'integer',
                minimum: 0,
                description: 'Override output size limit in bytes.',
              },
              env: {
                type: 'object',
                additionalProperties: { type: 'string' },
                description: 'Extra environment variables.',
              },
            },
            required: ['command', 'intent', 'target'],
          },
        },
      },
      {
        type: 'function',
        function: {
          name: 'list_targets',
          description: 'List available targets configured in octovalve-proxy.',
          parameters: {
            type: 'object',
            additionalProperties: false,
            properties: {},
            required: [],
          },
        },
      },
    ];
  }

  async function refreshOpenaiTools(targets: TargetInfo[]) {
    if (!openaiInitialized.value) {
      return false;
    }
    const targetNames = buildTargetNameList(targets);
    const defaultTarget = resolveDefaultTarget(targets, targetNames);
    const signature = buildOpenaiToolsSignature(targetNames, defaultTarget);
    if (signature === openaiToolsSignature.value) {
      return false;
    }
    try {
      await openaiService.setTools(buildOpenaiTools(targetNames, defaultTarget));
      openaiToolsSignature.value = signature;
      return true;
    } catch (e) {
      console.warn('[chatStore] refreshOpenaiTools failed:', e);
      return false;
    }
  }

  function normalizeSendOptions(input: string | SendMessageOptions): SendMessageOptions {
    if (typeof input === 'string') {
      return { content: input };
    }
    return {
      content: input.content ?? '',
      images: input.images ?? [],
      blocks: input.blocks,
      files: input.files ?? [],
      context: input.context,
    };
  }

  function buildPromptBlocks(options: SendMessageOptions): PromptBlock[] {
    if (options.blocks && options.blocks.length > 0) {
      return options.blocks;
    }
    const blocks: PromptBlock[] = [];
    const text = options.content?.trim();
    if (text) {
      blocks.push({ type: 'text', text });
    }
    if (options.images) {
      for (const image of options.images) {
        blocks.push({
          type: 'image',
          data: image.data,
          mimeType: image.mimeType,
          previewUrl: image.previewUrl,
        });
      }
    }
    if (options.files) {
      for (const file of options.files) {
        blocks.push({
          type: 'text',
          text: `[File: ${file.name}]\n${file.content}`,
        });
      }
    }
    return blocks;
  }

  function toAcpPromptBlocks(blocks: PromptBlock[]): AcpContentBlock[] {
    return blocks
      .map((block) => {
        if (block.type === 'text') {
          return { type: 'text', text: block.text } as const;
        }
        if (block.type === 'image') {
          return {
            type: 'image',
            data: block.data,
            mime_type: block.mimeType,
          } as const;
        }
        return null;
      })
      .filter((block): block is AcpContentBlock => block !== null);
  }

  function toOpenAiContentParts(blocks: PromptBlock[]): OpenAiContentPart[] {
    return blocks
      .map((block) => {
        if (block.type === 'text') {
          return { type: 'text', text: block.text } as const;
        }
        if (block.type === 'image') {
          const url = block.previewUrl ?? `data:${block.mimeType};base64,${block.data}`;
          return { type: 'image_url', image_url: { url } } as const;
        }
        return null;
      })
      .filter((part): part is OpenAiContentPart => part !== null);
  }

  function toDisplayImages(images?: ImageAttachment[]): string[] | undefined {
    if (!images || images.length === 0) {
      return undefined;
    }
    return images.map((img) => img.previewUrl);
  }

  function toDisplayFiles(files?: TextAttachment[]): string[] | undefined {
    if (!files || files.length === 0) {
      return undefined;
    }
    return files.map((file) => file.name);
  }

  function buildTextPrompt(options: SendMessageOptions): string {
    const parts: string[] = [];
    const text = options.content?.trim();
    if (text) {
      parts.push(text);
    }
    if (options.files) {
      for (const file of options.files) {
        parts.push(`[File: ${file.name}]\n${file.content}`);
      }
    }
    return parts.join('\n\n');
  }

  async function sendOpenaiMessage(options: SendMessageOptions) {
    const content = options.content ?? '';
    if (!openaiInitialized.value) {
      throw new Error('OpenAI not initialized');
    }

    const blocks = buildPromptBlocks(options);
    const openaiContent = toOpenAiContentParts(blocks);
    if (openaiContent.length === 0) {
      return;
    }

    const closed = closePendingOpenaiToolCalls(t('chat.tool.closeReason.beforeSend'));
    if (closed) {
      await syncOpenaiContextForSession(activeSession.value);
    } else {
      await ensureOpenaiContextForActiveSession();
    }

    await openaiContextQueue;

    // Add user message to UI
    addMessage({
      type: 'say',
      say: 'text',
      role: 'user',
      content,
      status: 'complete',
      images: toDisplayImages(options.images),
      files: toDisplayFiles(options.files),
    });

    // Add user message to OpenAI context
    await enqueueOpenaiContextOp(async () => {
      await openaiService.addMessage({ role: 'user', content: openaiContent });
    });

    await openaiContextQueue;

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
    currentAssistantStreamProvider = 'openai';

    setStreaming(true);

    try {
      await openaiService.send();
    } catch (e) {
      updateMessage(assistantMsg.id, { status: 'error', content: `Error: ${e}` });
      setStreaming(false);
      currentAssistantMessageId.value = null;
      currentAssistantStreamProvider = null;
      throw e;
    }
  }

  function setupOpenaiEventListener() {
    openaiListenerToken += 1;
    const token = openaiListenerToken;
    if (openaiEventUnlisten) {
      openaiEventUnlisten();
      openaiEventUnlisten = null;
    }
    openaiService.onStream(handleOpenaiStreamEvent).then((unlisten) => {
      if (openaiListenerToken !== token) {
        unlisten();
        return;
      }
      openaiEventUnlisten = unlisten;
    });
  }

  function handleOpenaiStreamEvent(event: ChatStreamEvent) {
    if (event.eventType === 'content' && event.content) {
      if (currentAssistantMessageId.value) {
        pendingAssistantContent += event.content;
        scheduleFlush();
      }
    } else if (event.eventType === 'reasoning' && event.content) {
      if (currentAssistantMessageId.value) {
        pendingAssistantReasoning += event.content;
        scheduleFlush();
      }
    } else if (event.eventType === 'tool_calls' && Array.isArray(event.toolCalls)) {
      flushPending();
      const messageId = currentAssistantMessageId.value;
      if (!messageId) {
        return;
      }

      for (const tc of event.toolCalls) {
        let args: Record<string, unknown> = {};
        const rawArgs = tc.function?.arguments ?? '';
        if (rawArgs && rawArgs.trim()) {
          try {
            args = JSON.parse(rawArgs) as Record<string, unknown>;
          } catch {
            args = { __raw: rawArgs };
          }
        }
        addToolCall(messageId, {
          id: tc.id,
          name: tc.function?.name || 'Unknown Tool',
          arguments: args,
          status: 'pending',
        });
      }

      updateMessage(messageId, { status: 'complete', partial: false });
      currentAssistantMessageId.value = null;
      currentAssistantStreamProvider = null;

      void handleOpenaiToolCalls(messageId, event.toolCalls);
    } else if (event.eventType === 'cancelled') {
      flushPending();
      if (currentAssistantMessageId.value) {
        const msg = activeSession.value?.messages.find(m => m.id === currentAssistantMessageId.value);
        if (msg && (!msg.content || !msg.content.trim())) {
          updateMessage(currentAssistantMessageId.value, { status: 'cancelled', content: t('chat.response.stopped'), partial: false });
        } else {
          updateMessage(currentAssistantMessageId.value, { status: 'cancelled', partial: false });
        }
        currentAssistantMessageId.value = null;
        currentAssistantStreamProvider = null;
      }
      setStreaming(false);
    } else if (event.eventType === 'complete') {
      flushPending();
      if (currentAssistantMessageId.value) {
        updateMessage(currentAssistantMessageId.value, { status: 'complete', partial: false });
        currentAssistantMessageId.value = null;
        currentAssistantStreamProvider = null;
      }
      setStreaming(false);
      scheduleSaveToStorage();
    } else if (event.eventType === 'error' && event.error) {
      flushPending();
      setError(event.error);
      if (currentAssistantMessageId.value) {
        updateMessage(currentAssistantMessageId.value, { status: 'error', content: event.error });
        currentAssistantMessageId.value = null;
        currentAssistantStreamProvider = null;
      }
      setStreaming(false);
    }
  }

  function extractToolResultText(payload: unknown): string {
    if (!payload) return '';
    if (typeof payload === 'string') return payload;
    if (typeof payload !== 'object') return String(payload);
    const value = payload as Record<string, unknown>;
    const content = value.content;
    if (Array.isArray(content) && content.length > 0) {
      const first = content[0] as Record<string, unknown> | undefined;
      const text = first?.text;
      if (typeof text === 'string') {
        return text;
      }
    }
    try {
      return JSON.stringify(payload, null, 2);
    } catch {
      return String(payload);
    }
  }

  async function handleOpenaiToolCalls(messageId: string, toolCalls: NonNullable<ChatStreamEvent['toolCalls']>) {
    setStreaming(true);
    const toolSignal = beginOpenaiToolRun();
    const cancelWait = toolCancelPromise(toolSignal);
    const results = new Map<string, string>();
    const queue = toolCalls.slice();
    const workerCount = Math.min(TOOL_CALL_CONCURRENCY_LIMIT, queue.length);

    const runWorker = async () => {
      while (queue.length > 0) {
        if (toolSignal.aborted) {
          return;
        }
        const tc = queue.shift();
        if (!tc) {
          return;
        }
        if (isToolCallCancelled(messageId, tc.id)) {
          continue;
        }
        const name = tc.function?.name || '';
        if (!name) {
          const text = 'Missing tool name';
          updateToolCall(messageId, tc.id, { status: 'failed', result: text });
          results.set(tc.id, text);
          continue;
        }

        let args: Record<string, unknown> = {};
        const rawArgs = tc.function?.arguments ?? '';
        if (rawArgs && rawArgs.trim()) {
          try {
            args = JSON.parse(rawArgs) as Record<string, unknown>;
          } catch {
            args = { __raw: rawArgs };
          }
        }

        updateToolCall(messageId, tc.id, { status: 'running' });
        const callPromise = openaiService
          .mcpCallTool(name, args)
          .then((result) => ({ ok: true as const, result }))
          .catch((error) => ({ ok: false as const, error }));
        const outcome = await Promise.race([callPromise, cancelWait]);
        if (outcome === 'cancelled' || toolSignal.aborted || isToolCallCancelled(messageId, tc.id)) {
          continue;
        }
        if (!outcome.ok) {
          const text = `Tool call failed: ${String(outcome.error)}`;
          updateToolCall(messageId, tc.id, { status: 'failed', result: text });
          results.set(tc.id, text);
          continue;
        }

        const text = extractToolResultText(outcome.result);
        updateToolCall(messageId, tc.id, { status: 'completed', result: text });
        results.set(tc.id, text);
      }
    };

    await Promise.all(Array.from({ length: workerCount }, () => runWorker()));

    if (toolSignal.aborted) {
      setStreaming(false);
      return;
    }

    for (const tc of toolCalls) {
      if (isToolCallCancelled(messageId, tc.id)) {
        continue;
      }
      const text = results.get(tc.id);
      if (!text) {
        continue;
      }
      await enqueueOpenaiContextOp(async () => {
        await openaiService.addMessage({
          role: 'tool',
          content: text,
          tool_call_id: tc.id,
        });
      });
    }

    await openaiContextQueue;

    const assistantMsg = addMessage({
      type: 'say',
      say: 'text',
      role: 'assistant',
      content: '',
      status: 'streaming',
      partial: true,
    });
    currentAssistantMessageId.value = assistantMsg.id;
    currentAssistantStreamProvider = 'openai';

    try {
      await openaiService.send();
    } catch (e) {
      updateMessage(assistantMsg.id, { status: 'error', content: `Error: ${e}` });
      setStreaming(false);
      currentAssistantMessageId.value = null;
      currentAssistantStreamProvider = null;
      return;
    }
  }

  async function cancelOpenai() {
    try {
      flushPending();
      abortOpenaiToolRun();
      await openaiService.cancel();
    } catch (e) {
      console.warn('[chatStore] openai cancel failed:', e);
    }
    setStreaming(false);
    if (currentAssistantMessageId.value) {
      const msg = activeSession.value?.messages.find(m => m.id === currentAssistantMessageId.value);
      if (msg && (!msg.content || !msg.content.trim())) {
        updateMessage(currentAssistantMessageId.value, { status: 'cancelled', content: t('chat.response.stopped'), partial: false });
      } else {
        updateMessage(currentAssistantMessageId.value, { status: 'cancelled', partial: false });
      }
      currentAssistantMessageId.value = null;
      currentAssistantStreamProvider = null;
    }
    if (closePendingOpenaiToolCalls(t('chat.tool.closeReason.userStop'))) {
      await syncOpenaiContextForSession(activeSession.value);
    }
  }

  async function stopOpenai() {
    console.log('[chatStore] stopOpenai called');
    openaiListenerToken += 1;
    if (openaiEventUnlisten) {
      openaiEventUnlisten();
      openaiEventUnlisten = null;
    }
    openaiInitialized.value = false;
    providerInitialized.value = false;
    setConnected(false);
    openaiContextSessionId.value = null;
    openaiToolsSignature.value = '';
    console.log('[chatStore] stopOpenai done');
  }

  watch(
    () => activeSessionId.value,
    () => {
      scheduleSaveToStorage();
      if (provider.value === 'openai') {
        openaiContextSessionId.value = null;
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

  // Unified send message
  async function sendMessage(input: string | SendMessageOptions, cwd?: string) {
    const options = normalizeSendOptions(input);
    if (provider.value === 'openai') {
      return sendOpenaiMessage(options);
    } else {
      return sendAcpMessage(options, cwd);
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
    sendOpenaiMessage,
    cancelOpenai,
    stopOpenai,
  };
});
