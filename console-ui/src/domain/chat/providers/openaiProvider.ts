import { ref, type ComputedRef, type Ref } from 'vue';
import type {
  ChatMessage,
  ChatProvider,
  ChatSession,
  SendMessageOptions,
  ToolCall,
} from '../types';
import type { TargetInfo } from '../../../shared/types';
import { buildMcpTools } from '../../../shared/mcpTools';
import {
  buildPromptBlocks,
  toDisplayFiles,
  toDisplayImages,
  toOpenAiContentParts,
} from '../pipeline/chatPipeline';
import type { ChatStreamEvent, OpenAiConfig, OpenAiContentPart } from '../services/openaiService';

type OpenaiService = {
  init: (config: OpenAiConfig) => Promise<void>;
  setTools: (tools: unknown[]) => Promise<void>;
  clearMessages: () => Promise<void>;
  addMessage: (message: unknown) => Promise<void>;
  send: () => Promise<void>;
  cancel: () => Promise<void>;
  onStream: (handler: (event: ChatStreamEvent) => void) => Promise<() => void>;
  mcpCallTool: (name: string, args: Record<string, unknown>) => Promise<unknown>;
};

type OpenaiProviderContext = {
  activeSession: ComputedRef<ChatSession | null>;
  activeSessionId: Ref<string | null>;
  provider: Ref<ChatProvider>;
  providerInitialized: Ref<boolean>;
  openaiInitialized: Ref<boolean>;
  isStreaming: Ref<boolean>;
  currentAssistantMessageId: Ref<string | null>;
  currentAssistantStreamProvider: Ref<ChatProvider | null>;
  setConnected: (value: boolean) => void;
  setStreaming: (value: boolean) => void;
  setError: (message: string | null) => void;
  addMessage: (message: Omit<ChatMessage, 'id' | 'ts'>) => ChatMessage;
  updateMessage: (messageId: string, updates: Partial<ChatMessage>) => void;
  scheduleSaveToStorage: () => void;
  addToolCall: (messageId: string, toolCall: ToolCall) => void;
  updateToolCall: (messageId: string, toolCallId: string, updates: Partial<ToolCall>) => void;
  isToolCallCancelled: (messageId: string, toolCallId: string) => boolean;
  flushPending: () => void;
  queueAssistantContent: (delta: string) => void;
  queueAssistantReasoning: (delta: string) => void;
};

type OpenaiProviderDeps = {
  openaiService: OpenaiService;
  fetchTargets: () => Promise<TargetInfo[]>;
  t: (key: string, params?: Record<string, unknown>) => string;
};

const TOOL_CALL_CONCURRENCY_LIMIT = 10;

export function createOpenaiProvider(context: OpenaiProviderContext, deps: OpenaiProviderDeps) {
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
        console.warn('[openaiProvider] context op failed:', e);
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

  function isFinalToolCallStatus(status: ToolCall['status']) {
    return status === 'completed' || status === 'failed' || status === 'cancelled';
  }

  function closePendingOpenaiToolCalls(reason: string): boolean {
    const session = context.activeSession.value;
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
              tc.result = deps.t('chat.tool.result.missing', { status: tc.status });
              changed = true;
            }
          }
          continue;
        }
        tc.status = 'cancelled';
        const result = (tc.result || '').trim();
        if (!result) {
          tc.result = reason;
        }
        changed = true;
      }
    }
    if (changed) {
      context.scheduleSaveToStorage();
    }
    return changed;
  }

  async function syncOpenaiContextForSession(session: ChatSession | null) {
    if (!context.openaiInitialized.value || context.provider.value !== 'openai') {
      return;
    }
    if (context.isStreaming.value) {
      return;
    }
    await enqueueOpenaiContextOp(async () => {
      await deps.openaiService.clearMessages();
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
            await deps.openaiService.addMessage({ role: 'assistant', content, tool_calls });
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
              await deps.openaiService.addMessage({ role: 'user', content: parts });
            }
          } else if (content) {
            await deps.openaiService.addMessage({ role: msg.role, content });
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
              await deps.openaiService.addMessage({
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
    const session = context.activeSession.value;
    if (!session) {
      return;
    }
    if (openaiContextSessionId.value === session.id) {
      return;
    }
    await syncOpenaiContextForSession(session);
  }

  function setupOpenaiEventListener() {
    openaiListenerToken += 1;
    const token = openaiListenerToken;
    if (openaiEventUnlisten) {
      openaiEventUnlisten();
      openaiEventUnlisten = null;
    }
    deps.openaiService.onStream(handleOpenaiStreamEvent).then((unlisten) => {
      if (openaiListenerToken !== token) {
        unlisten();
        return;
      }
      openaiEventUnlisten = unlisten;
    });
  }

  function handleOpenaiStreamEvent(event: ChatStreamEvent) {
    if (event.eventType === 'content' && event.content) {
      if (context.currentAssistantMessageId.value) {
        context.queueAssistantContent(event.content);
      }
    } else if (event.eventType === 'reasoning' && event.content) {
      if (context.currentAssistantMessageId.value) {
        context.queueAssistantReasoning(event.content);
      }
    } else if (event.eventType === 'tool_calls' && Array.isArray(event.toolCalls)) {
      context.flushPending();
      const messageId = context.currentAssistantMessageId.value;
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
        context.addToolCall(messageId, {
          id: tc.id,
          name: tc.function?.name || 'Unknown Tool',
          arguments: args,
          status: 'pending',
        });
      }

      context.updateMessage(messageId, { status: 'complete', partial: false });
      context.currentAssistantMessageId.value = null;
      context.currentAssistantStreamProvider.value = null;

      void handleOpenaiToolCalls(messageId, event.toolCalls);
    } else if (event.eventType === 'cancelled') {
      context.flushPending();
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
      context.setStreaming(false);
    } else if (event.eventType === 'complete') {
      context.flushPending();
      if (context.currentAssistantMessageId.value) {
        context.updateMessage(context.currentAssistantMessageId.value, { status: 'complete', partial: false });
        context.currentAssistantMessageId.value = null;
        context.currentAssistantStreamProvider.value = null;
      }
      context.setStreaming(false);
      context.scheduleSaveToStorage();
    } else if (event.eventType === 'error' && event.error) {
      context.flushPending();
      context.setError(event.error);
      if (context.currentAssistantMessageId.value) {
        context.updateMessage(context.currentAssistantMessageId.value, { status: 'error', content: event.error });
        context.currentAssistantMessageId.value = null;
        context.currentAssistantStreamProvider.value = null;
      }
      context.setStreaming(false);
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

  async function handleOpenaiToolCalls(
    messageId: string,
    toolCalls: NonNullable<ChatStreamEvent['toolCalls']>
  ) {
    context.setStreaming(true);
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
        if (context.isToolCallCancelled(messageId, tc.id)) {
          continue;
        }
        const name = tc.function?.name || '';
        if (!name) {
          const text = 'Missing tool name';
          context.updateToolCall(messageId, tc.id, { status: 'failed', result: text });
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

        context.updateToolCall(messageId, tc.id, { status: 'running' });
        const callPromise = deps.openaiService
          .mcpCallTool(name, args)
          .then((result) => ({ ok: true as const, result }))
          .catch((error) => ({ ok: false as const, error }));
        const outcome = await Promise.race([callPromise, cancelWait]);
        if (outcome === 'cancelled' || toolSignal.aborted || context.isToolCallCancelled(messageId, tc.id)) {
          continue;
        }
        if (!outcome.ok) {
          const text = `Tool call failed: ${String(outcome.error)}`;
          context.updateToolCall(messageId, tc.id, { status: 'failed', result: text });
          results.set(tc.id, text);
          continue;
        }

        const text = extractToolResultText(outcome.result);
        context.updateToolCall(messageId, tc.id, { status: 'completed', result: text });
        results.set(tc.id, text);
      }
    };

    await Promise.all(Array.from({ length: workerCount }, () => runWorker()));

    if (toolSignal.aborted) {
      context.setStreaming(false);
      return;
    }

    for (const tc of toolCalls) {
      if (context.isToolCallCancelled(messageId, tc.id)) {
        continue;
      }
      const text = results.get(tc.id);
      if (!text) {
        continue;
      }
      await enqueueOpenaiContextOp(async () => {
        await deps.openaiService.addMessage({
          role: 'tool',
          content: text,
          tool_call_id: tc.id,
        });
      });
    }

    await openaiContextQueue;

    const assistantMsg = context.addMessage({
      type: 'say',
      say: 'text',
      role: 'assistant',
      content: '',
      status: 'streaming',
      partial: true,
    });
    context.currentAssistantMessageId.value = assistantMsg.id;
    context.currentAssistantStreamProvider.value = 'openai';

    try {
      await deps.openaiService.send();
    } catch (e) {
      context.updateMessage(assistantMsg.id, { status: 'error', content: `Error: ${e}` });
      context.setStreaming(false);
      context.currentAssistantMessageId.value = null;
      context.currentAssistantStreamProvider.value = null;
    }
  }

  async function initializeOpenai(config: OpenAiConfig) {
    console.log('[chatStore] initializeOpenai called');
    try {
      console.log('[chatStore] initializeOpenai: calling openaiService.init()');
      await deps.openaiService.init(config);
      console.log('[chatStore] initializeOpenai: openaiService.init() done');
      openaiToolsSignature.value = '';

      try {
        await deps.openaiService.setTools(buildOpenaiTools([], undefined));
        openaiToolsSignature.value = buildOpenaiToolsSignature([], undefined);
      } catch (e) {
        console.warn('[openaiProvider] setTools base failed (continuing without tools):', e);
      }

      try {
        const targets = await deps.fetchTargets();
        const targetNames = buildTargetNameList(targets);
        const defaultTarget = resolveDefaultTarget(targets, targetNames);
        const signature = buildOpenaiToolsSignature(targetNames, defaultTarget);
        if (signature !== openaiToolsSignature.value) {
          await deps.openaiService.setTools(buildOpenaiTools(targetNames, defaultTarget));
          openaiToolsSignature.value = signature;
        }
      } catch (e) {
        console.warn('[openaiProvider] refresh tools from targets failed:', e);
      }

      context.openaiInitialized.value = true;
      context.providerInitialized.value = true;
      context.provider.value = 'openai';
      setupOpenaiEventListener();
      context.setConnected(true);
      await syncOpenaiContextForSession(context.activeSession.value);
      console.log('[chatStore] initializeOpenai done');
    } catch (e) {
      console.error('[chatStore] initializeOpenai failed:', e);
      context.setError(`Failed to initialize OpenAI: ${e}`);
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
    return buildMcpTools(targets, defaultTarget);
  }

  async function refreshOpenaiTools(targets: TargetInfo[]) {
    if (!context.openaiInitialized.value || context.provider.value !== 'openai') {
      return;
    }
    const targetNames = buildTargetNameList(targets);
    const defaultTarget = resolveDefaultTarget(targets, targetNames);
    const signature = buildOpenaiToolsSignature(targetNames, defaultTarget);
    if (signature === openaiToolsSignature.value) {
      return;
    }
    try {
      await deps.openaiService.setTools(buildOpenaiTools(targetNames, defaultTarget));
      openaiToolsSignature.value = signature;
    } catch (e) {
      console.warn('[openaiProvider] refreshOpenaiTools failed:', e);
    }
  }

  async function sendOpenaiMessage(options: SendMessageOptions) {
    const content = options.content ?? '';
    if (!context.openaiInitialized.value) {
      throw new Error('OpenAI not initialized');
    }

    const blocks = buildPromptBlocks(options);
    const openaiContent = toOpenAiContentParts(blocks);
    if (openaiContent.length === 0) {
      return;
    }

    const closed = closePendingOpenaiToolCalls(deps.t('chat.tool.closeReason.beforeSend'));
    if (closed) {
      await syncOpenaiContextForSession(context.activeSession.value);
    } else {
      await ensureOpenaiContextForActiveSession();
    }

    await openaiContextQueue;

    context.addMessage({
      type: 'say',
      say: 'text',
      role: 'user',
      content,
      status: 'complete',
      images: toDisplayImages(options.images),
      files: toDisplayFiles(options.files),
    });

    await enqueueOpenaiContextOp(async () => {
      await deps.openaiService.addMessage({ role: 'user', content: openaiContent });
    });

    await openaiContextQueue;

    const assistantMsg = context.addMessage({
      type: 'say',
      say: 'text',
      role: 'assistant',
      content: '',
      status: 'streaming',
      partial: true,
    });
    context.currentAssistantMessageId.value = assistantMsg.id;
    context.currentAssistantStreamProvider.value = 'openai';
    context.setStreaming(true);

    try {
      await deps.openaiService.send();
    } catch (e) {
      context.updateMessage(assistantMsg.id, { status: 'error', content: `Error: ${e}` });
      context.setStreaming(false);
      context.currentAssistantMessageId.value = null;
      context.currentAssistantStreamProvider.value = null;
    }
  }

  async function cancelOpenai() {
    try {
      context.flushPending();
      abortOpenaiToolRun();
      await deps.openaiService.cancel();
    } catch (e) {
      console.warn('[openaiProvider] cancel failed:', e);
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
    if (closePendingOpenaiToolCalls(deps.t('chat.tool.closeReason.userStop'))) {
      await syncOpenaiContextForSession(context.activeSession.value);
    }
  }

  async function stopOpenai() {
    console.log('[chatStore] stopOpenai called');
    openaiListenerToken += 1;
    if (openaiEventUnlisten) {
      openaiEventUnlisten();
      openaiEventUnlisten = null;
    }
    context.openaiInitialized.value = false;
    context.providerInitialized.value = false;
    context.setConnected(false);
    openaiContextSessionId.value = null;
    openaiToolsSignature.value = '';
    console.log('[chatStore] stopOpenai done');
  }

  function resetContextForSessionChange() {
    openaiContextSessionId.value = null;
  }

  async function clearOpenaiContext() {
    openaiContextSessionId.value = null;
    if (!context.openaiInitialized.value || context.provider.value !== 'openai') {
      return;
    }
    await enqueueOpenaiContextOp(async () => {
      await deps.openaiService.clearMessages();
    });
  }

  return {
    initializeOpenai,
    refreshOpenaiTools,
    sendOpenaiMessage,
    cancelOpenai,
    stopOpenai,
    resetContextForSessionChange,
    clearOpenaiContext,
  };
}
