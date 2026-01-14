import { invoke } from '@tauri-apps/api/core';
import type { McpToolInfo } from '../../../shared/mcpTools';

export async function mcpSetConfig(configJson: string): Promise<void> {
  return invoke('mcp_set_config', { configJson });
}

export async function mcpListTools(): Promise<McpToolInfo[]> {
  return invoke<McpToolInfo[]>('mcp_list_tools');
}

export async function mcpCallTool(
  server: string,
  name: string,
  args: Record<string, unknown>
): Promise<unknown> {
  return invoke('mcp_call_tool', { server, name, arguments: args });
}

export const mcpService = {
  setConfig: mcpSetConfig,
  listTools: mcpListTools,
  callTool: mcpCallTool,
};
