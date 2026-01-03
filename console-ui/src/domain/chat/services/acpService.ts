/**
 * ACP (Agent Client Protocol) service for communicating with codex-acp backend
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

// ============================================================================
// Types
// ============================================================================

export interface AgentInfo {
  name: string;
  version: string;
  title?: string;
}

export interface AuthMethod {
  id: string;
  name: string;
  description?: string;
}

export interface AgentCapabilities {
  loadSession?: boolean;
  promptCapabilities?: unknown;
  mcpCapabilities?: unknown;
}

export interface AcpInitResponse {
  agentInfo: AgentInfo | null;
  authMethods: AuthMethod[];
  agentCapabilities?: AgentCapabilities | null;
}

export interface SessionMode {
  id: string;
  name: string;
  description?: string;
}

export interface SessionModel {
  id: string;
  name: string;
  description?: string;
}

export interface AcpSessionInfo {
  sessionId: string;
  modes: SessionMode[];
  models: SessionModel[];
}

export interface AcpLoadSessionResult {
  modes: unknown;
  models: unknown;
  history: unknown;
}

export interface ContextItem {
  type: string;
  [key: string]: unknown;
}

export interface AcpEvent {
  type: string;
  payload: unknown;
}

export interface ContentDeltaPayload {
  session_id: string;
  content: string;
}

export interface ToolCallStartPayload {
  session_id: string;
  tool_call_id: string;
  name: string;
  arguments?: unknown;
}

export interface ToolCallEndPayload {
  session_id: string;
  tool_call_id: string;
  result?: string;
  error?: string;
}

export interface PermissionRequestPayload {
  session_id: string;
  request_id: string;
  kind: string;
  command?: string;
  cwd?: string;
  path?: string;
  diff?: string;
}

export interface ErrorPayload {
  session_id: string;
  message: string;
}

export interface CompletePayload {
  session_id: string;
  stop_reason: string;
}

// ============================================================================
// Service
// ============================================================================

/**
 * Start the ACP client and initialize connection
 */
export async function acpStart(cwd: string, codexAcpPath?: string): Promise<AcpInitResponse> {
  return invoke<AcpInitResponse>('acp_start', { cwd, codexAcpPath });
}

/**
 * Authenticate with the agent using specified method
 */
export async function acpAuthenticate(methodId: string): Promise<void> {
  return invoke('acp_authenticate', { methodId });
}

/**
 * Create a new session
 */
export async function acpNewSession(cwd: string): Promise<AcpSessionInfo> {
  return invoke<AcpSessionInfo>('acp_new_session', { cwd });
}

/**
 * Load an existing session
 */
export async function acpLoadSession(sessionId: string): Promise<AcpLoadSessionResult> {
  return invoke<AcpLoadSessionResult>('acp_load_session', { sessionId });
}

/**
 * Send a prompt to the current session
 */
export async function acpPrompt(content: string, context?: ContextItem[]): Promise<void> {
  return invoke('acp_prompt', { content, context });
}

/**
 * Cancel the current operation
 */
export async function acpCancel(): Promise<void> {
  return invoke('acp_cancel');
}

/**
 * Stop the ACP client
 */
export async function acpStop(): Promise<void> {
  return invoke('acp_stop');
}

/**
 * Listen for ACP events
 */
export function onAcpEvent(callback: (event: AcpEvent) => void): Promise<UnlistenFn> {
  return listen<AcpEvent>('acp-event', (e) => {
    callback(e.payload);
  });
}

// ============================================================================
// Convenience wrapper
// ============================================================================

export const acpService = {
  start: acpStart,
  authenticate: acpAuthenticate,
  newSession: acpNewSession,
  loadSession: acpLoadSession,
  prompt: acpPrompt,
  cancel: acpCancel,
  stop: acpStop,
  onEvent: onAcpEvent,
};

export default acpService;
