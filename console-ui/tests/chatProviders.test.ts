import { computed, ref } from 'vue';
import { describe, expect, it, vi } from 'vitest';
import type { ChatMessage, ChatProvider, ChatSession } from '../src/domain/chat/types';
import type { AcpSessionSummary, AgentCapabilities, AuthMethod } from '../src/domain/chat/services/acpService';
import { createAcpProvider } from '../src/domain/chat/providers/acpProvider';
import { createOpenaiProvider } from '../src/domain/chat/providers/openaiProvider';

describe('chatProviders', () => {
  it('openaiProvider cancels streaming message with stopped text', async () => {
    const sessions = ref<ChatSession[]>([]);
    const activeSessionId = ref<string | null>('s1');
    const provider = ref<ChatProvider>('openai');
    const providerInitialized = ref(true);
    const openaiInitialized = ref(true);
    const isStreaming = ref(true);
    const currentAssistantMessageId = ref<string | null>(null);
    const currentAssistantStreamProvider = ref<ChatProvider | null>('openai');
    const error = ref<string | null>(null);
    const isConnected = ref(true);

    const message: ChatMessage = {
      id: 'm1',
      ts: 1,
      type: 'say',
      say: 'text',
      role: 'assistant',
      content: '',
      status: 'streaming',
    };

    const session: ChatSession = {
      id: 's1',
      provider: 'openai',
      title: 'Test',
      createdAt: 0,
      updatedAt: 0,
      messages: [message],
      totalTokens: 0,
      status: 'running',
    };

    sessions.value.push(session);
    currentAssistantMessageId.value = message.id;

    const activeSession = computed(() => sessions.value.find((s) => s.id === activeSessionId.value) ?? null);

    const addMessage = vi.fn();
    const updateMessage = (messageId: string, updates: Partial<ChatMessage>) => {
      const msg = session.messages.find((m) => m.id === messageId);
      if (msg) {
        Object.assign(msg, updates);
      }
    };

    const openaiService = {
      init: vi.fn(),
      setTools: vi.fn(),
      clearMessages: vi.fn(),
      addMessage: vi.fn(),
      send: vi.fn(),
      cancel: vi.fn().mockResolvedValue(undefined),
      onStream: vi.fn().mockResolvedValue(() => {}),
      mcpCallTool: vi.fn(),
    };

    const providerApi = createOpenaiProvider(
      {
        activeSession,
        activeSessionId,
        provider,
        providerInitialized,
        openaiInitialized,
        isStreaming,
        currentAssistantMessageId,
        currentAssistantStreamProvider,
        setConnected: (value) => {
          isConnected.value = value;
        },
        setStreaming: (value) => {
          isStreaming.value = value;
        },
        setError: (message) => {
          error.value = message;
        },
        addMessage,
        updateMessage,
        scheduleSaveToStorage: vi.fn(),
        addToolCall: vi.fn(),
        updateToolCall: vi.fn(),
        isToolCallCancelled: () => false,
        flushPending: vi.fn(),
        queueAssistantContent: vi.fn(),
        queueAssistantReasoning: vi.fn(),
      },
      {
        openaiService,
        fetchTargets: vi.fn().mockResolvedValue([]),
        t: (key) => `t:${key}`,
      }
    );

    await providerApi.cancelOpenai();

    expect(openaiService.cancel).toHaveBeenCalledOnce();
    expect(isStreaming.value).toBe(false);
    expect(currentAssistantMessageId.value).toBeNull();
    expect(currentAssistantStreamProvider.value).toBeNull();
    expect(message.status).toBe('cancelled');
    expect(message.content).toBe('t:chat.response.stopped');
    expect(error.value).toBeNull();
  });

  it('acpProvider initializes connection state', async () => {
    const sessions = ref<ChatSession[]>([]);
    const activeSessionId = ref<string | null>(null);
    const provider = ref<ChatProvider>('openai');
    const providerInitialized = ref(false);
    const acpInitialized = ref(false);
    const authMethods = ref<AuthMethod[]>([]);
    const acpCapabilities = ref<AgentCapabilities | null>(null);
    const acpCwd = ref<string | null>(null);
    const acpLoadedSessionId = ref<string | null>(null);
    const acpHistorySummaries = ref<AcpSessionSummary[]>([]);
    const acpHistoryLoading = ref(false);
    const currentAssistantMessageId = ref<string | null>(null);
    const currentAssistantStreamProvider = ref<ChatProvider | null>(null);
    const isStreaming = ref(false);
    const isConnected = ref(false);
    const error = ref<string | null>(null);

    const activeSession = computed(() => sessions.value.find((s) => s.id === activeSessionId.value) ?? null);
    const createSession = (title?: string) => {
      const session: ChatSession = {
        id: `s-${sessions.value.length}`,
        provider: 'openai',
        title: title ?? 'New',
        createdAt: 0,
        updatedAt: 0,
        messages: [],
        totalTokens: 0,
        status: 'idle',
      };
      sessions.value.push(session);
      activeSessionId.value = session.id;
      return session;
    };

    const acpService = {
      start: vi.fn().mockResolvedValue({
        authMethods: [{ id: 'token', name: 'Token' }],
        agentCapabilities: { loadSession: true },
      }),
      authenticate: vi.fn(),
      newSession: vi.fn(),
      loadSession: vi.fn(),
      listSessions: vi.fn(),
      deleteSession: vi.fn(),
      prompt: vi.fn(),
      cancel: vi.fn(),
      stop: vi.fn(),
      onEvent: vi.fn().mockResolvedValue(() => {}),
    };

    const providerApi = createAcpProvider(
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
        setConnected: (value) => {
          isConnected.value = value;
        },
        setStreaming: vi.fn(),
        setError: (message) => {
          error.value = message;
        },
        createSession,
        addMessage: vi.fn(),
        updateMessage: vi.fn(),
        scheduleSaveToStorage: vi.fn(),
        saveToStorage: vi.fn(),
        addToolCall: vi.fn(),
        updateToolCall: vi.fn(),
        findToolCall: vi.fn(),
        flushPending: vi.fn(),
        queueAssistantContent: vi.fn(),
        queueAssistantReasoning: vi.fn(),
        generateId: () => 'id',
      },
      {
        acpService,
        t: (key) => `t:${key}`,
      }
    );

    await providerApi.initializeAcp('/tmp');

    expect(acpService.start).toHaveBeenCalledWith('/tmp', undefined);
    expect(acpInitialized.value).toBe(true);
    expect(provider.value).toBe('acp');
    expect(providerInitialized.value).toBe(true);
    expect(authMethods.value).toEqual([{ id: 'token', name: 'Token' }]);
    expect(acpCapabilities.value).toEqual({ loadSession: true });
    expect(acpCwd.value).toBe('/tmp');
    expect(acpLoadedSessionId.value).toBeNull();
    expect(isConnected.value).toBe(true);
    expect(error.value).toBeNull();
  });
});
