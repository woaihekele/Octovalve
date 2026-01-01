import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

export interface OpenAiConfig {
  baseUrl: string;
  apiKey: string;
  model: string;
  chatPath?: string;
}

export interface ChatMessage {
  role: 'user' | 'assistant' | 'system' | 'tool';
  content: string;
  toolCalls?: ToolCall[];
  toolCallId?: string;
}

export interface ToolCall {
  id: string;
  callType: string;
  function: {
    name: string;
    arguments: string;
  };
}

export interface Tool {
  toolType: string;
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

/**
 * Initialize the OpenAI client with configuration
 */
export async function openaiInit(config: OpenAiConfig): Promise<void> {
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
  return invoke('openai_send');
}

/**
 * Cancel the current streaming request
 */
export async function openaiCancel(): Promise<void> {
  return invoke('openai_cancel');
}

/**
 * Listen to OpenAI stream events
 */
export async function onOpenaiStream(
  callback: (event: ChatStreamEvent) => void
): Promise<UnlistenFn> {
  return listen<ChatStreamEvent>('openai-stream', (event) => {
    callback(event.payload);
  });
}

// Convenience service object
export const openaiService = {
  init: openaiInit,
  addMessage: openaiAddMessage,
  setTools: openaiSetTools,
  clearMessages: openaiClearMessages,
  send: openaiSend,
  cancel: openaiCancel,
  onStream: onOpenaiStream,
};
