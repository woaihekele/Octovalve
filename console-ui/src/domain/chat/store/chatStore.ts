import { defineStore } from 'pinia';
import { ref, computed, watch } from 'vue';
import type { ChatSession, ChatMessage, ChatConfig, ToolCall } from '../types';
import type { AuthMethod, AcpEvent, AgentCapabilities } from '../services/acpService';
import { acpService } from '../services/acpService';
import { openaiService, type OpenAiConfig, type ChatStreamEvent } from '../services/openaiService';
import { fetchTargets } from '../../../services/api';
import type { TargetInfo } from '../../../shared/types';
import { i18n } from '../../../i18n';

const t = i18n.global.t;

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

  // Provider state
  const provider = ref<ChatProvider>('openai');
  const providerInitialized = ref(false);

  // ACP state
  const acpInitialized = ref(false);
  const authMethods = ref<AuthMethod[]>([]);
  const acpCapabilities = ref<AgentCapabilities | null>(null);
  const currentAssistantMessageId = ref<string | null>(null);
  let acpEventUnlisten: (() => void) | null = null;

  let pendingAssistantContent = '';
  let pendingAssistantReasoning = '';
  let flushTimer: ReturnType<typeof setTimeout> | null = null;
  const flushIntervalMs = 50;

  const flushPending = () => {
    if (!currentAssistantMessageId.value || !activeSession.value) {
      pendingAssistantContent = '';
      pendingAssistantReasoning = '';
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
      updateMessage(messageId, { reasoning: (msg.reasoning || '') + delta });
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
      session.updatedAt = Date.now();
      saveToStorage();
    }
  }

  let acpHistoryLoadToken = 0;

  async function loadAcpSessionOrThrow(sessionId: string) {
    try {
      return await acpService.loadSession(sessionId);
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
    try {
      const stored = localStorage.getItem('octovalve-chat-sessions');
      if (stored) {
        const data = JSON.parse(stored);
        const rawSessions = (data.sessions ?? []) as ChatSession[];
        sessions.value = rawSessions.map((s) => ({
          ...s,
          provider: (s as any).provider === 'acp' ? 'acp' : 'openai',
        }));
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

  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleSaveToStorage() {
    if (typeof window === 'undefined' || typeof localStorage === 'undefined') {
      return;
    }
    if (saveTimer) {
      return;
    }
    saveTimer = setTimeout(() => {
      saveTimer = null;
      saveToStorage();
    }, 400);
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
  async function initializeAcp(cwd: string, codexAcpPath?: string, acpArgs?: string) {
    console.log('[chatStore] initializeAcp called with cwd:', cwd);
    try {
      console.log('[chatStore] initializeAcp: calling acpService.start...');
      const response = await acpService.start(cwd, codexAcpPath, acpArgs);
      console.log('[chatStore] initializeAcp: acpService.start returned:', response);
      authMethods.value = response.authMethods;
      acpInitialized.value = true;
      acpCapabilities.value = response.agentCapabilities ?? null;
      provider.value = 'acp';
      providerInitialized.value = true;
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
      if (canLoadAcpSession()) {
        const loaded = await loadAcpSessionOrThrow(session.acpSessionId);
        applyAcpHistoryToActiveSession(loaded.history);
      }
      return session.acpSessionId;
    }

    const info = await acpService.newSession(cwd);
    session.acpSessionId = info.sessionId;
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

  async function sendAcpMessage(content: string, cwd = '.') {
    if (!acpInitialized.value) {
      throw new Error('ACP not initialized');
    }

    await ensureAcpSessionLoaded(cwd);

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
    providerInitialized.value = false;
    setConnected(false);
    console.log('[chatStore] stopAcp done');
  }

  function canLoadAcpSession() {
    return acpCapabilities.value?.loadSession !== false;
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
      
      if (sessionUpdate === 'agent_message_chunk') {
        const content = update.content as { text?: string; type?: string } | undefined;
        if (content?.text && currentAssistantMessageId.value) {
          const contentType = content.type;
          if (contentType === 'reasoning' || contentType === 'reasoning_content' || contentType === 'thinking') {
            pendingAssistantReasoning += content.text;
            scheduleFlush();
          } else {
            pendingAssistantContent += content.text;
            scheduleFlush();
          }
        }
      } else if (sessionUpdate === 'tool_call') {
        // Tool call started
        const toolCallId = update.toolCallId as string;
        const title = update.title as string | undefined;
        const name = update.name as string | undefined;
        if (currentAssistantMessageId.value && toolCallId) {
          addToolCall(currentAssistantMessageId.value, {
            id: toolCallId,
            name: name || title || 'Unknown Tool',
            arguments: {},
            status: 'running',
          });
        }
      } else if (sessionUpdate === 'tool_call_update') {
        // Tool call output/status update
        const toolCallId = update.toolCallId as string;
        const content = update.content as { text?: string } | undefined;
        const status = update.status as string | undefined;
        if (currentAssistantMessageId.value && toolCallId) {
          const updates: Partial<ToolCall> = {};
          if (content?.text) {
            updates.result = (updates.result || '') + content.text;
          }
          if (status === 'completed' || status === 'done') {
            updates.status = 'completed';
          } else if (status === 'failed' || status === 'error') {
            updates.status = 'failed';
          }
          updateToolCall(currentAssistantMessageId.value, toolCallId, updates);
        }
      } else if (sessionUpdate === 'task_complete') {
        flushPending();
        if (currentAssistantMessageId.value) {
          updateMessage(currentAssistantMessageId.value, { status: 'complete', partial: false });
          currentAssistantMessageId.value = null;
        }
        setStreaming(false);
      } else if (sessionUpdate === 'error') {
        flushPending();
        const errorMsg = (update.error as { message?: string })?.message || 'Unknown error';
        setError(errorMsg);
        if (currentAssistantMessageId.value) {
          updateMessage(currentAssistantMessageId.value, { status: 'error', content: errorMsg });
          currentAssistantMessageId.value = null;
        }
        setStreaming(false);
      }
    } else if (method === 'prompt/complete') {
      // Handle prompt completion (stopReason: "end_turn")
      console.log('[chatStore] prompt/complete received:', payload);
      flushPending();
      if (currentAssistantMessageId.value) {
        updateMessage(currentAssistantMessageId.value, { status: 'complete', partial: false });
        currentAssistantMessageId.value = null;
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

  async function sendOpenaiMessage(content: string) {
    if (!openaiInitialized.value) {
      throw new Error('OpenAI not initialized');
    }

    if (!content.trim()) {
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
    });

    // Add user message to OpenAI context
    await enqueueOpenaiContextOp(async () => {
      await openaiService.addMessage({ role: 'user', content });
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

    setStreaming(true);

    try {
      await openaiService.send();
    } catch (e) {
      updateMessage(assistantMsg.id, { status: 'error', content: `Error: ${e}` });
      setStreaming(false);
      currentAssistantMessageId.value = null;
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
          status: 'running',
        });
      }

      updateMessage(messageId, { status: 'complete', partial: false });
      currentAssistantMessageId.value = null;

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
      }
      setStreaming(false);
    } else if (event.eventType === 'complete') {
      flushPending();
      if (currentAssistantMessageId.value) {
        updateMessage(currentAssistantMessageId.value, { status: 'complete', partial: false });
        currentAssistantMessageId.value = null;
      }
      setStreaming(false);
      scheduleSaveToStorage();
    } else if (event.eventType === 'error' && event.error) {
      flushPending();
      setError(event.error);
      if (currentAssistantMessageId.value) {
        updateMessage(currentAssistantMessageId.value, { status: 'error', content: event.error });
        currentAssistantMessageId.value = null;
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

    for (const tc of toolCalls) {
      if (toolSignal.aborted || isToolCallCancelled(messageId, tc.id)) {
        break;
      }
      const name = tc.function?.name || '';
      if (!name) {
        updateToolCall(messageId, tc.id, { status: 'failed', result: 'Missing tool name' });
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

      const callPromise = openaiService
        .mcpCallTool(name, args)
        .then((result) => ({ ok: true as const, result }))
        .catch((error) => ({ ok: false as const, error }));
      const outcome = await Promise.race([callPromise, cancelWait]);
      if (outcome === 'cancelled' || toolSignal.aborted || isToolCallCancelled(messageId, tc.id)) {
        break;
      }
      if (!outcome.ok) {
        const text = `Tool call failed: ${String(outcome.error)}`;
        updateToolCall(messageId, tc.id, { status: 'failed', result: text });
        await enqueueOpenaiContextOp(async () => {
          await openaiService.addMessage({
            role: 'tool',
            content: text,
            tool_call_id: tc.id,
          });
        });
        continue;
      }

      const text = extractToolResultText(outcome.result);
      updateToolCall(messageId, tc.id, { status: 'completed', result: text });
      await enqueueOpenaiContextOp(async () => {
        await openaiService.addMessage({
          role: 'tool',
          content: text,
          tool_call_id: tc.id,
        });
      });
    }

    if (toolSignal.aborted) {
      setStreaming(false);
      return;
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

    try {
      await openaiService.send();
    } catch (e) {
      updateMessage(assistantMsg.id, { status: 'error', content: `Error: ${e}` });
      setStreaming(false);
      currentAssistantMessageId.value = null;
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
  async function sendMessage(content: string, cwd = '.') {
    if (provider.value === 'openai') {
      return sendOpenaiMessage(content);
    } else {
      return sendAcpMessage(content, cwd);
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
    sendMessage,
    // ACP
    acpInitialized,
    authMethods,
    initializeAcp,
    authenticateAcp,
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
