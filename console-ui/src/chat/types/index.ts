/**
 * Chat message types - inspired by Cline's message system
 * Simplified for initial implementation
 */

export type MessageRole = 'user' | 'assistant' | 'system';

export type MessageStatus = 'pending' | 'streaming' | 'complete' | 'error' | 'cancelled';

export type AskType =
  | 'followup'
  | 'command'
  | 'tool'
  | 'api_req_failed'
  | 'resume_task';

export type SayType =
  | 'task'
  | 'text'
  | 'reasoning'
  | 'error'
  | 'command'
  | 'command_output'
  | 'tool'
  | 'api_req_started'
  | 'api_req_finished'
  | 'completion_result'
  | 'user_feedback';

export interface ToolCall {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
  status: 'pending' | 'running' | 'completed' | 'failed';
  result?: string;
}

export interface ChatMessage {
  id: string;
  ts: number;
  type: 'ask' | 'say';
  ask?: AskType;
  say?: SayType;
  role: MessageRole;
  content: string;
  reasoning?: string;
  status: MessageStatus;
  toolCalls?: ToolCall[];
  images?: string[];
  files?: string[];
  partial?: boolean;
  modelInfo?: {
    model: string;
    tokensIn?: number;
    tokensOut?: number;
    cost?: number;
  };
}

export interface ChatSession {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  messages: ChatMessage[];
  totalTokens: number;
  status: 'idle' | 'running' | 'paused';
}

export interface ChatState {
  sessions: ChatSession[];
  activeSessionId: string | null;
  isConnected: boolean;
  isStreaming: boolean;
  error: string | null;
}

export interface SendMessageOptions {
  content: string;
  images?: string[];
  files?: string[];
  context?: Record<string, unknown>;
}

export interface ChatConfig {
  systemPrompt?: string;
  greeting?: string;
  model?: string;
  maxTokens?: number;
  temperature?: number;
}
