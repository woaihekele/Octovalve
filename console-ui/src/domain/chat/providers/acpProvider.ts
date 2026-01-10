import type { ComputedRef, Ref } from 'vue';
import type {
  ChatMessage,
  ChatProvider,
  ChatSession,
  PlanEntry,
  PlanEntryPriority,
  PlanEntryStatus,
  SendMessageOptions,
  ToolCall,
} from '../types';
import {
  buildPromptBlocks,
  parseAcpHistory,
  toAcpPromptBlocks,
  toDisplayFiles,
  toDisplayImages,
} from '../pipeline/chatPipeline';
import type {
  AcpEvent,
  AcpSessionSummary,
  AgentCapabilities,
  AuthMethod,
} from '../services/acpService';
import { concatAcpTextChunk, ensureToolCallBlock } from '../store/acpTimeline';

type AcpService = {
  start: (cwd: string, args?: string) => Promise<{ authMethods: AuthMethod[]; agentCapabilities?: AgentCapabilities }>;
  authenticate: (methodId: string) => Promise<void>;
  newSession: (cwd: string) => Promise<{ sessionId: string }>;
  loadSession: (sessionId: string) => Promise<{ history: unknown }>;
  listSessions: () => Promise<{ sessions?: AcpSessionSummary[] }>;
  deleteSession: (sessionId: string) => Promise<void>;
  prompt: (prompt: unknown[], context?: Array<{ type: string; [key: string]: unknown }>) => Promise<void>;
  cancel: () => Promise<void>;
  stop: () => Promise<void>;
  onEvent: (callback: (event: AcpEvent) => void) => Promise<() => void>;
};

type AcpProviderContext = {
  sessions: Ref<ChatSession[]>;
  activeSessionId: Ref<string | null>;
  activeSession: ComputedRef<ChatSession | null>;
  provider: Ref<ChatProvider>;
  providerInitialized: Ref<boolean>;
  acpInitialized: Ref<boolean>;
  authMethods: Ref<AuthMethod[]>;
  acpCapabilities: Ref<AgentCapabilities | null>;
  acpCwd: Ref<string | null>;
  acpLoadedSessionId: Ref<string | null>;
  acpHistorySummaries: Ref<AcpSessionSummary[]>;
  acpHistoryLoading: Ref<boolean>;
  currentAssistantMessageId: Ref<string | null>;
  currentAssistantStreamProvider: Ref<ChatProvider | null>;
  isStreaming: Ref<boolean>;
  setConnected: (value: boolean) => void;
  setStreaming: (value: boolean) => void;
  setError: (message: string | null) => void;
  createSession: (title?: string) => ChatSession;
  addMessage: (message: Omit<ChatMessage, 'id' | 'ts'>) => ChatMessage;
  updateMessage: (messageId: string, updates: Partial<ChatMessage>) => void;
  scheduleSaveToStorage: () => void;
  saveToStorage: () => void;
  addToolCall: (messageId: string, toolCall: ToolCall) => void;
  updateToolCall: (messageId: string, toolCallId: string, updates: Partial<ToolCall>) => void;
  findToolCall: (messageId: string, toolCallId: string) => ToolCall | undefined;
  flushPending: () => void;
  queueAssistantContent: (delta: string) => void;
  queueAssistantReasoning: (delta: string, merge?: boolean) => void;
  generateId: () => string;
};

type AcpProviderDeps = {
  acpService: AcpService;
  t: (key: string, params?: Record<string, unknown>) => string;
};

export function createAcpProvider(context: AcpProviderContext, deps: AcpProviderDeps) {
  let acpEventUnlisten: (() => void) | null = null;
  let acpHistoryLoadToken = 0;

  function applyAcpHistoryToActiveSession(history: unknown) {
    const session = context.activeSession.value;
    if (!session) return;

    const parsed = parseAcpHistory(history, context.generateId);
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
      context.saveToStorage();
    }
  }

  async function loadAcpSessionOrThrow(sessionId: string) {
    try {
      const loaded = await deps.acpService.loadSession(sessionId);
      context.acpLoadedSessionId.value = sessionId;
      return loaded;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      context.setError(`Failed to load ACP session: ${msg}`);
      throw e;
    }
  }

  async function maybeLoadAcpHistoryForActiveSession() {
    if (!context.acpInitialized.value) {
      return;
    }
    if (context.provider.value !== 'acp') {
      return;
    }
    if (!canLoadAcpSession()) {
      return;
    }
    if (context.isStreaming.value) {
      return;
    }
    const session = context.activeSession.value;
    const sessionId = session?.acpSessionId;
    if (!sessionId) {
      return;
    }
    if (context.acpLoadedSessionId.value === sessionId) {
      return;
    }
    const token = ++acpHistoryLoadToken;
    try {
      const loaded = await loadAcpSessionOrThrow(sessionId);
      if (token !== acpHistoryLoadToken) {
        return;
      }
      applyAcpHistoryToActiveSession(loaded.history);
    } catch {
      // error already reported
    }
  }

  async function refreshAcpHistorySummaries() {
    if (!context.acpInitialized.value) {
      context.acpHistorySummaries.value = [];
      return;
    }
    if (context.acpHistoryLoading.value) {
      return;
    }
    context.acpHistoryLoading.value = true;
    try {
      const result = await deps.acpService.listSessions();
      context.acpHistorySummaries.value = result.sessions ?? [];
    } catch (e) {
      console.warn('[acpProvider] load ACP history failed:', e);
      context.acpHistorySummaries.value = [];
    } finally {
      context.acpHistoryLoading.value = false;
    }
  }

  function findAcpSessionByRemoteId(sessionId: string) {
    return context.sessions.value.find((s) => s.provider === 'acp' && s.acpSessionId === sessionId);
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
      context.activeSessionId.value = existing.id;
      context.scheduleSaveToStorage();
      return;
    }

    const newSession = context.createSession(summary?.title);
    newSession.provider = 'acp';
    newSession.acpSessionId = sessionId;
    if (summary?.messageCount) {
      newSession.messageCount = summary.messageCount;
    }
    if (summary?.updatedAt) {
      newSession.updatedAt = summary.updatedAt;
    }
    context.activeSessionId.value = newSession.id;
    context.scheduleSaveToStorage();
  }

  async function deleteAcpHistorySession(sessionId: string) {
    const active = context.activeSession.value;
    const deletingActive = active?.provider === 'acp' && active.acpSessionId === sessionId;

    if (deletingActive) {
      await cancelAcp();
      context.currentAssistantMessageId.value = null;
      context.currentAssistantStreamProvider.value = null;
      context.acpLoadedSessionId.value = null;
    } else if (context.acpLoadedSessionId.value === sessionId) {
      context.acpLoadedSessionId.value = null;
    }

    await deps.acpService.deleteSession(sessionId);
    context.acpHistorySummaries.value = context.acpHistorySummaries.value.filter(
      (item) => item.sessionId !== sessionId
    );
    const activeId = context.activeSessionId.value;
    context.sessions.value = context.sessions.value.filter(
      (session) => !(session.provider === 'acp' && session.acpSessionId === sessionId)
    );
    if (deletingActive) {
      context.activeSessionId.value = null;
      context.createSession();
    } else if (activeId && !context.sessions.value.find((session) => session.id === activeId)) {
      context.activeSessionId.value = context.sessions.value[0]?.id ?? null;
      if (!context.activeSessionId.value && context.provider.value === 'acp') {
        context.createSession();
      }
    }
    context.scheduleSaveToStorage();
  }

  async function clearAcpHistorySessions() {
    const active = context.activeSession.value;
    if (active?.provider === 'acp') {
      await cancelAcp();
      context.currentAssistantMessageId.value = null;
      context.currentAssistantStreamProvider.value = null;
      context.acpLoadedSessionId.value = null;
    }

    const summaries = [...context.acpHistorySummaries.value];
    for (const item of summaries) {
      await deps.acpService.deleteSession(item.sessionId);
    }
    context.acpHistorySummaries.value = [];
    context.sessions.value = context.sessions.value.filter((session) => session.provider !== 'acp');
    if (context.provider.value === 'acp') {
      context.activeSessionId.value = null;
      context.createSession();
    } else if (
      context.activeSessionId.value &&
      !context.sessions.value.find((s) => s.id === context.activeSessionId.value)
    ) {
      context.activeSessionId.value = context.sessions.value[0]?.id ?? null;
    }
    context.scheduleSaveToStorage();
  }

  async function initializeAcp(cwd: string, acpArgs?: string) {
    console.log('[chatStore] initializeAcp called with cwd:', cwd);
    try {
      console.log('[chatStore] initializeAcp: calling acpService.start...');
      const response = await deps.acpService.start(cwd, acpArgs);
      console.log('[chatStore] initializeAcp: acpService.start returned:', response);
      context.authMethods.value = response.authMethods;
      context.acpInitialized.value = true;
      context.setConnected(true);
      context.acpCapabilities.value = response.agentCapabilities ?? null;
      context.provider.value = 'acp';
      context.providerInitialized.value = true;
      context.acpCwd.value = cwd;
      context.acpLoadedSessionId.value = null;
      setupAcpEventListener();
      if (context.activeSession.value?.acpSessionId) {
        try {
          if (canLoadAcpSession()) {
            const loaded = await loadAcpSessionOrThrow(context.activeSession.value.acpSessionId);
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
      context.setError(`Failed to initialize ACP: ${e}`);
      context.setConnected(false);
      throw e;
    }
  }

  async function authenticateAcp(methodId: string) {
    try {
      await deps.acpService.authenticate(methodId);
      context.setConnected(true);
    } catch (e) {
      context.setConnected(false);
      context.setError(`Authentication failed: ${e}`);
      throw e;
    }
  }

  async function ensureAcpSessionLoaded(cwd: string): Promise<string> {
    if (!context.activeSession.value) {
      context.createSession();
    }
    const session = context.activeSession.value!;

    if (session.acpSessionId) {
      if (canLoadAcpSession() && context.acpLoadedSessionId.value !== session.acpSessionId) {
        const loaded = await loadAcpSessionOrThrow(session.acpSessionId);
        applyAcpHistoryToActiveSession(loaded.history);
      }
      return session.acpSessionId;
    }

    const info = await deps.acpService.newSession(cwd);
    session.acpSessionId = info.sessionId;
    context.acpLoadedSessionId.value = info.sessionId;
    session.updatedAt = Date.now();
    context.saveToStorage();
    return info.sessionId;
  }

  async function sendAcpMessage(options: SendMessageOptions, cwd?: string) {
    if (!context.acpInitialized.value) {
      throw new Error('ACP not initialized');
    }

    const resolvedCwd = cwd ?? context.acpCwd.value ?? '.';
    await ensureAcpSessionLoaded(resolvedCwd);

    const promptBlocks = buildPromptBlocks(options);
    const acpPromptBlocks = toAcpPromptBlocks(promptBlocks);
    if (acpPromptBlocks.length === 0) {
      return;
    }

    const content = options.content ?? '';
    context.addMessage({
      type: 'say',
      say: 'text',
      role: 'user',
      content,
      status: 'complete',
      images: toDisplayImages(options.images),
      files: toDisplayFiles(options.files),
    });

    const assistantMsg = context.addMessage({
      type: 'say',
      say: 'text',
      role: 'assistant',
      content: '',
      status: 'streaming',
      partial: true,
      blocks: [],
    });
    context.currentAssistantMessageId.value = assistantMsg.id;
    context.currentAssistantStreamProvider.value = 'acp';

    context.setStreaming(true);

    try {
      await deps.acpService.prompt(acpPromptBlocks, options.context);
    } catch (e) {
      context.updateMessage(assistantMsg.id, { status: 'error', content: `Error: ${e}` });
      context.setStreaming(false);
      context.currentAssistantMessageId.value = null;
      context.currentAssistantStreamProvider.value = null;
      throw e;
    }
  }

  async function cancelAcp() {
    try {
      context.flushPending();
      await deps.acpService.cancel();
    } catch (e) {
      console.warn('[acpProvider] cancel failed:', e);
    }
    context.setStreaming(false);
    if (context.currentAssistantMessageId.value) {
      const msg = context.activeSession.value?.messages.find(
        (m) => m.id === context.currentAssistantMessageId.value
      );
      if (msg && (!msg.content || !msg.content.trim())) {
        context.updateMessage(context.currentAssistantMessageId.value, {
          status: 'cancelled',
          content: deps.t('chat.response.stopped'),
          partial: false,
        });
      } else {
        context.updateMessage(context.currentAssistantMessageId.value, { status: 'cancelled', partial: false });
      }
      context.currentAssistantMessageId.value = null;
      context.currentAssistantStreamProvider.value = null;
    }
  }

  async function stopAcp() {
    try {
      if (acpEventUnlisten) {
        acpEventUnlisten();
        acpEventUnlisten = null;
      }
      if (context.acpInitialized.value) {
        await deps.acpService.stop();
      }
    } catch (e) {
      console.warn('[acpProvider] stop failed:', e);
    }
    context.acpInitialized.value = false;
    context.acpCapabilities.value = null;
    context.acpCwd.value = null;
    context.acpLoadedSessionId.value = null;
    context.providerInitialized.value = false;
    context.setConnected(false);
  }

  function canLoadAcpSession() {
    return context.acpCapabilities.value?.loadSession !== false;
  }

  function setupAcpEventListener() {
    if (acpEventUnlisten) {
      acpEventUnlisten();
      acpEventUnlisten = null;
    }
    deps.acpService.onEvent(handleAcpEvent).then((unlisten) => {
      acpEventUnlisten = unlisten;
    });
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
    if (!Array.isArray(value)) return [];
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

    if (context.provider.value !== 'acp') {
      return;
    }

    const activeAcpSessionId = context.activeSession.value?.acpSessionId ?? null;
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

      if (
        sessionUpdate === 'agent_message_chunk' ||
        sessionUpdate === 'agent_thought_chunk' ||
        sessionUpdate === 'agent_reasoning_chunk'
      ) {
        const contentText = extractContentText(update.content);
        if (contentText && context.currentAssistantMessageId.value) {
          if (sessionUpdate === 'agent_thought_chunk' || sessionUpdate === 'agent_reasoning_chunk') {
            context.queueAssistantReasoning(contentText, true);
          } else {
            context.queueAssistantContent(contentText);
          }
        }
      } else if (sessionUpdate === 'tool_call') {
        context.flushPending();
        const messageId = context.currentAssistantMessageId.value;
        if (!messageId) {
          return;
        }
        const toolCallId = (update.toolCallId ?? update.tool_call_id) as string | undefined;
        const title = update.title as string | undefined;
        const name = update.name as string | undefined;
        const rawInput = update.rawInput ?? update.raw_input;
        const status = mapAcpToolStatus(update.status);
        if (toolCallId && !context.findToolCall(messageId, toolCallId)) {
          context.addToolCall(messageId, {
            id: toolCallId,
            name: name || title || 'Unknown Tool',
            arguments: normalizeToolArguments(rawInput),
            status: status ?? 'running',
          });
        }
        if (toolCallId) {
          const session = context.activeSession.value;
          if (!session) {
            return;
          }
          const msg = session.messages.find((m) => m.id === messageId);
          if (msg) {
            const { blocks, inserted } = ensureToolCallBlock(msg.blocks, toolCallId);
            if (inserted) {
              context.updateMessage(messageId, { blocks });
            } else {
              session.updatedAt = Date.now();
              context.scheduleSaveToStorage();
            }
          }
        }
      } else if (sessionUpdate === 'tool_call_update') {
        const toolCallId = (update.toolCallId ?? update.tool_call_id) as string | undefined;
        if (context.currentAssistantMessageId.value && toolCallId) {
          const existing = context.findToolCall(context.currentAssistantMessageId.value, toolCallId);
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
            context.flushPending();
            context.addToolCall(context.currentAssistantMessageId.value, {
              id: toolCallId,
              name: updates.name || 'Unknown Tool',
              arguments: updates.arguments ?? {},
              status: updates.status ?? 'running',
              result: updates.result,
            });
          } else {
            context.updateToolCall(context.currentAssistantMessageId.value, toolCallId, updates);
          }
          const session = context.activeSession.value;
          if (!session) {
            return;
          }
          const msg = session.messages.find((m) => m.id === context.currentAssistantMessageId.value);
          if (msg) {
            const { blocks, inserted } = ensureToolCallBlock(msg.blocks, toolCallId);
            if (inserted) {
              context.updateMessage(context.currentAssistantMessageId.value, { blocks });
            }
            session.updatedAt = Date.now();
            context.scheduleSaveToStorage();
          }
        }
      } else if (sessionUpdate === 'retry') {
        const attempt = (update.attempt as number | undefined) ?? 0;
        const maxAttempts =
          (update.maxAttempts as number | undefined) ?? (update.max_attempts as number | undefined) ?? 0;
        const message = (update.message as string | undefined) ?? 'Retrying...';
        if (context.currentAssistantMessageId.value) {
          const retryToolCallId = 'acp-retry';
          if (!context.findToolCall(context.currentAssistantMessageId.value, retryToolCallId)) {
            context.flushPending();
            context.addToolCall(context.currentAssistantMessageId.value, {
              id: retryToolCallId,
              name: 'Retry',
              arguments: { attempt, maxAttempts },
              status: 'running',
            });
          }
          context.updateToolCall(context.currentAssistantMessageId.value, retryToolCallId, {
            status: 'running',
            arguments: { attempt, maxAttempts },
            result: `[${attempt}/${maxAttempts}] ${message}\n`,
          });
          const session = context.activeSession.value;
          if (!session) {
            return;
          }
          const msg = session.messages.find((m) => m.id === context.currentAssistantMessageId.value);
          if (msg) {
            const { blocks, inserted } = ensureToolCallBlock(msg.blocks, retryToolCallId);
            if (inserted) {
              context.updateMessage(context.currentAssistantMessageId.value, { blocks });
            }
            session.updatedAt = Date.now();
            context.scheduleSaveToStorage();
          }
        }
      } else if (sessionUpdate === 'plan') {
        const rawEntries =
          (update.entries as unknown) ??
          ((update.plan as Record<string, unknown> | undefined)?.entries as unknown);
        const entries = normalizePlanEntries(rawEntries);
        if (context.activeSession.value) {
          context.activeSession.value.plan = entries;
          context.activeSession.value.updatedAt = Date.now();
          context.scheduleSaveToStorage();
        }
      } else if (sessionUpdate === 'task_complete') {
        context.flushPending();
        if (context.currentAssistantMessageId.value) {
          const retryToolCallId = 'acp-retry';
          const existingRetry = context.findToolCall(context.currentAssistantMessageId.value, retryToolCallId);
          if (existingRetry && existingRetry.status === 'running') {
            context.updateToolCall(context.currentAssistantMessageId.value, retryToolCallId, {
              status: 'completed',
            });
          }
          context.updateMessage(context.currentAssistantMessageId.value, { status: 'complete', partial: false });
          context.currentAssistantMessageId.value = null;
          context.currentAssistantStreamProvider.value = null;
        }
        context.setStreaming(false);
      } else if (sessionUpdate === 'error') {
        context.flushPending();
        const errorMsg = (update.error as { message?: string })?.message || 'Unknown error';
        context.setError(errorMsg);
        if (context.currentAssistantMessageId.value) {
          const retryToolCallId = 'acp-retry';
          const existingRetry = context.findToolCall(context.currentAssistantMessageId.value, retryToolCallId);
          if (existingRetry && existingRetry.status === 'running') {
            context.updateToolCall(context.currentAssistantMessageId.value, retryToolCallId, { status: 'failed' });
          }
          context.updateMessage(context.currentAssistantMessageId.value, { status: 'error', content: errorMsg });
          context.currentAssistantMessageId.value = null;
          context.currentAssistantStreamProvider.value = null;
        }
        context.setStreaming(false);
      }
    } else if (method === 'prompt/complete') {
      console.log('[chatStore] prompt/complete received:', payload);
      context.flushPending();
      if (context.currentAssistantMessageId.value) {
        const retryToolCallId = 'acp-retry';
        const existingRetry = context.findToolCall(context.currentAssistantMessageId.value, retryToolCallId);
        if (existingRetry && existingRetry.status === 'running') {
          context.updateToolCall(context.currentAssistantMessageId.value, retryToolCallId, { status: 'completed' });
        }
        context.updateMessage(context.currentAssistantMessageId.value, { status: 'complete', partial: false });
        context.currentAssistantMessageId.value = null;
        context.currentAssistantStreamProvider.value = null;
      }
      context.setStreaming(false);
    }
  }

  return {
    initializeAcp,
    authenticateAcp,
    refreshAcpHistorySummaries,
    activateAcpHistorySession,
    deleteAcpHistorySession,
    clearAcpHistorySessions,
    sendAcpMessage,
    cancelAcp,
    stopAcp,
    maybeLoadAcpHistoryForActiveSession,
  };
}
