import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

export interface OpenAiConfig {
  baseUrl: string;
  apiKey: string;
  model: string;
  chatPath?: string;
}

export type OpenAiContentPart =
  | { type: 'text'; text: string }
  | {
      type: 'image_url';
      image_url: {
        url: string;
        detail?: 'auto' | 'low' | 'high';
      };
    };

export type OpenAiMessageContent = string | OpenAiContentPart[];

export interface ChatMessage {
  role: 'user' | 'assistant' | 'system' | 'tool';
  content: OpenAiMessageContent;
  tool_calls?: ToolCall[];
  tool_call_id?: string;
}

export interface ToolCall {
  id: string;
  type: string;
  function: {
    name: string;
    arguments: string;
  };
}

export interface Tool {
  type: string;
  function: {
    name: string;
    description: string;
    parameters: Record<string, unknown>;
  };
}

export interface ChatStreamEvent {
  eventType: 'content' | 'reasoning' | 'tool_calls' | 'complete' | 'cancelled' | 'error';
  content?: string;
  toolCalls?: ToolCall[];
  finishReason?: string;
  error?: string;
}

type OpenaiStreamCallback = (event: ChatStreamEvent) => void;
type OpenaiStreamBridge = {
  callbacks: Set<OpenaiStreamCallback>;
  unlisten: UnlistenFn | null;
  listening: boolean;
  pending: Promise<void> | null;
};

const OPENAI_STREAM_BRIDGE_KEY = '__octovalveOpenaiStreamBridge';

function logUiEvent(message: string) {
  void invoke('log_ui_event', { message });
}

// Keep a single OpenAI stream listener across HMR reloads.
function getOpenaiStreamBridge(): OpenaiStreamBridge {
  const global = globalThis as typeof globalThis & {
    [OPENAI_STREAM_BRIDGE_KEY]?: OpenaiStreamBridge;
  };
  if (!global[OPENAI_STREAM_BRIDGE_KEY]) {
    global[OPENAI_STREAM_BRIDGE_KEY] = {
      callbacks: new Set(),
      unlisten: null,
      listening: false,
      pending: null,
    };
  }
  return global[OPENAI_STREAM_BRIDGE_KEY] as OpenaiStreamBridge;
}

async function ensureOpenaiStreamListener() {
  const bridge = getOpenaiStreamBridge();
  if (bridge.listening) {
    return;
  }
  if (bridge.pending) {
    await bridge.pending;
    return;
  }
  bridge.pending = listen<ChatStreamEvent>('openai-stream', (event) => {
    const activeBridge = getOpenaiStreamBridge();
    if (activeBridge.callbacks.size === 0) {
      return;
    }
    for (const callback of Array.from(activeBridge.callbacks)) {
      try {
        callback(event.payload);
      } catch (err) {
        console.warn('[openaiService] stream callback failed:', err);
      }
    }
  })
    .then((unlisten) => {
      const activeBridge = getOpenaiStreamBridge();
      activeBridge.unlisten = unlisten;
      activeBridge.listening = true;
      activeBridge.pending = null;
    })
    .catch((err) => {
      const activeBridge = getOpenaiStreamBridge();
      activeBridge.pending = null;
      activeBridge.listening = false;
      throw err;
    });
  await bridge.pending;
}

/**
 * Initialize the OpenAI client with configuration
 */
export async function openaiInit(config: OpenAiConfig): Promise<void> {
  logUiEvent(
    `[openaiService] init baseUrl=${config.baseUrl} chatPath=${config.chatPath ?? ''} model=${config.model} apiKeyLen=${config.apiKey?.length ?? 0}`
  );
  return invoke('openai_init', { config });
}

/**
 * Add a message to the conversation history
 */
export async function openaiAddMessage(message: ChatMessage): Promise<void> {
  return invoke('openai_add_message', { message });
}

/**
 * Set the available tools for the model
 */
export async function openaiSetTools(tools: Tool[]): Promise<void> {
  return invoke('openai_set_tools', { tools });
}

/**
 * Clear all messages from conversation history
 */
export async function openaiClearMessages(): Promise<void> {
  return invoke('openai_clear_messages');
}

/**
 * Send the conversation and stream the response
 */
export async function openaiSend(): Promise<void> {
  logUiEvent('[openaiService] send');
  return invoke('openai_send');
}

/**
 * Cancel the current streaming request
 */
export async function openaiCancel(): Promise<void> {
  return invoke('openai_cancel');
}

export async function mcpCallTool(name: string, arguments_: Record<string, unknown>): Promise<unknown> {
  return invoke('mcp_call_tool', { name, arguments: arguments_ });
}

/**
 * Listen to OpenAI stream events
 */
export async function onOpenaiStream(
  callback: OpenaiStreamCallback
): Promise<UnlistenFn> {
  const bridge = getOpenaiStreamBridge();
  bridge.callbacks.add(callback);
  await ensureOpenaiStreamListener();
  return () => {
    const activeBridge = getOpenaiStreamBridge();
    activeBridge.callbacks.delete(callback);
    if (activeBridge.callbacks.size === 0 && activeBridge.unlisten) {
      activeBridge.unlisten();
      activeBridge.unlisten = null;
      activeBridge.listening = false;
    }
  };
}

// Convenience service object
export const openaiService = {
  init: openaiInit,
  addMessage: openaiAddMessage,
  setTools: openaiSetTools,
  clearMessages: openaiClearMessages,
  send: openaiSend,
  cancel: openaiCancel,
  mcpCallTool,
  onStream: onOpenaiStream,
};
