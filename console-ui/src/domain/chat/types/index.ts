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
  status: 'pending' | 'running' | 'completed' | 'failed' | 'cancelled';
  result?: string;
}

export interface ImageAttachment {
  kind: 'image';
  data: string;
  mimeType: string;
  previewUrl: string;
  name?: string;
  size?: number;
}

export interface TextAttachment {
  kind: 'text';
  name: string;
  mimeType: string;
  content: string;
  size?: number;
}

export type PromptBlock =
  | { type: 'text'; text: string }
  | { type: 'image'; data: string; mimeType: string; previewUrl?: string };

export type PlanEntryStatus = 'pending' | 'in_progress' | 'completed';
export type PlanEntryPriority = 'low' | 'medium' | 'high';

export interface PlanEntry {
  content: string;
  status: PlanEntryStatus;
  priority: PlanEntryPriority;
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
  provider: 'acp' | 'openai';
  title: string;
  createdAt: number;
  updatedAt: number;
  messages: ChatMessage[];
  totalTokens: number;
  status: 'idle' | 'running' | 'paused';
  acpSessionId?: string | null;
  plan?: PlanEntry[];
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
  images?: ImageAttachment[];
  blocks?: PromptBlock[];
  files?: TextAttachment[];
  context?: Array<{ type: string; [key: string]: unknown }>;
}

export interface ChatConfig {
  systemPrompt?: string;
  greeting?: string;
  model?: string;
  maxTokens?: number;
  temperature?: number;
}
