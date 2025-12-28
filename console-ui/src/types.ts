export type TargetStatus = 'ready' | 'down';

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
  is_default?: boolean;
}

export type CommandMode = 'shell' | 'argv';
export type CommandStatus = 'approved' | 'denied' | 'error' | 'completed';

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
  history: ResultSnapshot[];
  last_result?: ResultSnapshot | null;
}

export type ConsoleEvent =
  | { type: 'targets_snapshot'; targets: TargetInfo[] }
  | { type: 'target_updated'; target: TargetInfo };

export type ListTab = 'pending' | 'history';

export interface AppSettings {
  notificationsEnabled: boolean;
  shortcuts: {
    prevTarget: string;
    nextTarget: string;
    jumpNextPending: string;
    approve: string;
    deny: string;
    fullScreen: string;
    toggleList: string;
  };
}
