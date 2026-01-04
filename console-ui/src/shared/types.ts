export type TargetStatus = 'ready' | 'down';
export type ThemeMode = 'system' | 'dark' | 'light' | 'darcula';

export interface ConfigFilePayload {
  path: string;
  exists: boolean;
  content: string;
}

export interface ProfileSummary {
  name: string;
}

export interface ProfilesStatus {
  current: string;
  profiles: ProfileSummary[];
}

export interface TargetInfo {
  name: string;
  hostname?: string | null;
  ip?: string | null;
  desc: string;
  status: TargetStatus;
  pending_count: number;
  last_seen?: string | null;
  last_error?: string | null;
  control_addr?: string | null;
  local_addr?: string | null;
  terminal_available?: boolean;
  is_default?: boolean;
}

export type CommandMode = 'shell' | 'argv';
export type CommandStatus = 'approved' | 'denied' | 'error' | 'cancelled' | 'completed';

export interface CommandStage {
  argv: string[];
}

export interface RequestSnapshot {
  id: string;
  client: string;
  target: string;
  peer: string;
  intent: string;
  mode: CommandMode;
  raw_command: string;
  pipeline: CommandStage[];
  cwd?: string | null;
  timeout_ms?: number | null;
  max_output_bytes?: number | null;
  received_at_ms: number;
}

export interface RunningSnapshot {
  id: string;
  client: string;
  target: string;
  peer: string;
  intent: string;
  mode: CommandMode;
  raw_command: string;
  pipeline: CommandStage[];
  cwd?: string | null;
  timeout_ms?: number | null;
  max_output_bytes?: number | null;
  received_at_ms: number;
  queued_for_secs: number;
  started_at_ms: number;
}

export interface ResultSnapshot {
  id: string;
  status: CommandStatus;
  exit_code?: number | null;
  error?: string | null;
  intent: string;
  mode: CommandMode;
  raw_command: string;
  pipeline: CommandStage[];
  cwd?: string | null;
  peer: string;
  queued_for_secs: number;
  finished_at_ms: number;
  stdout?: string | null;
  stderr?: string | null;
}

export interface ServiceSnapshot {
  queue: RequestSnapshot[];
  running: RunningSnapshot[];
  history: ResultSnapshot[];
  last_result?: ResultSnapshot | null;
}

export type ConsoleEvent =
  | { type: 'targets_snapshot'; targets: TargetInfo[] }
  | { type: 'target_updated'; target: TargetInfo };

export type ListTab = 'pending' | 'history';

export type AiRiskLevel = 'low' | 'medium' | 'high';
export type AiRiskStatus = 'pending' | 'done' | 'error';

export interface AiRiskEntry {
  status: AiRiskStatus;
  risk?: AiRiskLevel;
  reason?: string;
  keyPoints?: string[];
  updatedAt: number;
  error?: string;
  autoApproved?: boolean;
  autoApprovedAt?: number;
}

export interface AiRiskApiResponse {
  risk: AiRiskLevel;
  reason: string;
  key_points: string[];
}

export interface AiSettings {
  enabled: boolean;
  autoApproveLowRisk: boolean;
  provider: 'openai';
  baseUrl: string;
  chatPath: string;
  model: string;
  apiKey: string;
  prompt: string;
  timeoutMs: number;
  maxConcurrency: number;
}

export interface ChatProviderConfig {
  provider: 'openai' | 'acp';
  sendOnEnter: boolean;
  openai: {
    baseUrl: string;
    apiKey: string;
    model: string;
    chatPath: string;
  };
  acp: {
    path: string;
  };
}

export interface AppSettings {
  notificationsEnabled: boolean;
  theme: ThemeMode;
  ai: AiSettings;
  chat: ChatProviderConfig;
  shortcuts: {
    prevTarget: string;
    nextTarget: string;
    jumpNextPending: string;
    approve: string;
    deny: string;
    fullScreen: string;
    openSettings: string;
  };
}
