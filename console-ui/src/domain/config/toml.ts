import type { BrokerConfigEditor, ProxyConfigEditor, ProxyTargetConfig } from '../../shared/types';

function isBlank(value: string | null | undefined) {
  return value === null || value === undefined || value.trim() === '';
}

function normalizeOptionalString(value: string | null | undefined) {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

function tomlString(value: string) {
  return JSON.stringify(value);
}

function writeStringArray(key: string, values: Array<string | null | undefined>): string[] {
  const filtered = values
    .map((value) => value?.trim())
    .filter((value): value is string => Boolean(value));
  if (filtered.length === 0) {
    return [`${key} = []`];
  }
  if (filtered.length <= 3 && filtered.join(', ').length < 56) {
    return [`${key} = [${filtered.map(tomlString).join(', ')}]`];
  }
  return [
    `${key} = [`,
    ...filtered.map((value) => `  ${tomlString(value)},`),
    `]`,
  ];
}

function writeInlineStringMap(key: string, map: Record<string, string> | null | undefined): string[] {
  if (!map) {
    return [];
  }
  const entries = Object.entries(map)
    .map(([k, v]) => [k.trim(), v] as const)
    .filter(([k, v]) => k && !isBlank(v));
  if (entries.length === 0) {
    return [];
  }
  entries.sort((a, b) => a[0].localeCompare(b[0]));
  const rendered = entries.map(([k, v]) => `${k} = ${tomlString(v)}`).join(', ');
  return [`${key} = { ${rendered} }`];
}

function pushIf(lines: string[], key: string, value: unknown) {
  if (value === null || value === undefined) {
    return;
  }
  if (typeof value === 'boolean') {
    if (value) {
      lines.push(`${key} = true`);
    }
    return;
  }
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) {
      return;
    }
    lines.push(`${key} = ${value}`);
    return;
  }
  if (typeof value === 'string') {
    const normalized = normalizeOptionalString(value);
    if (!normalized) {
      return;
    }
    lines.push(`${key} = ${tomlString(normalized)}`);
    return;
  }
}

function serializeTarget(target: ProxyTargetConfig): string[] {
  const lines: string[] = [];
  lines.push('[[targets]]');
  lines.push(`name = ${tomlString(target.name ?? '')}`);
  lines.push(`desc = ${tomlString(target.desc ?? '')}`);

  pushIf(lines, 'hostname', target.hostname);
  pushIf(lines, 'ip', target.ip);
  pushIf(lines, 'ssh', target.ssh);
  pushIf(lines, 'remote_addr', target.remote_addr);
  pushIf(lines, 'local_port', target.local_port);
  pushIf(lines, 'local_bind', target.local_bind);

  const sshArgs = target.ssh_args ?? [];
  if (Array.isArray(sshArgs) && sshArgs.length > 0) {
    lines.push(...writeStringArray('ssh_args', sshArgs));
  }
  pushIf(lines, 'ssh_password', target.ssh_password);
  pushIf(lines, 'terminal_locale', target.terminal_locale);
  pushIf(lines, 'tty', target.tty);
  pushIf(lines, 'control_remote_addr', target.control_remote_addr);
  pushIf(lines, 'control_local_port', target.control_local_port);
  pushIf(lines, 'control_local_bind', target.control_local_bind);

  return lines;
}

export function serializeProxyConfigToml(config: ProxyConfigEditor): string {
  const lines: string[] = [];
  const brokerPath = normalizeOptionalString(config.broker_config_path ?? undefined);
  if (brokerPath) {
    lines.push(`broker_config_path = ${tomlString(brokerPath)}`);
  }

  const defaultTarget = normalizeOptionalString(config.default_target ?? undefined);
  if (defaultTarget) {
    lines.push(`default_target = ${tomlString(defaultTarget)}`);
  }

  const defaults = config.defaults ?? null;
  if (defaults) {
    const defaultsLines: string[] = [];
    pushIf(defaultsLines, 'timeout_ms', defaults.timeout_ms);
    pushIf(defaultsLines, 'max_output_bytes', defaults.max_output_bytes);
    pushIf(defaultsLines, 'local_bind', defaults.local_bind);
    pushIf(defaultsLines, 'remote_addr', defaults.remote_addr);
    pushIf(defaultsLines, 'control_remote_addr', defaults.control_remote_addr);
    const sshArgs = defaults.ssh_args ?? [];
    if (Array.isArray(sshArgs) && sshArgs.length > 0) {
      defaultsLines.push(...writeStringArray('ssh_args', sshArgs));
    }
    pushIf(defaultsLines, 'ssh_password', defaults.ssh_password);
    pushIf(defaultsLines, 'terminal_locale', defaults.terminal_locale);
    pushIf(defaultsLines, 'control_local_bind', defaults.control_local_bind);
    pushIf(defaultsLines, 'control_local_port_offset', defaults.control_local_port_offset);
    if (defaultsLines.length > 0) {
      if (lines.length > 0) {
        lines.push('');
      }
      lines.push('[defaults]');
      lines.push(...defaultsLines);
    }
  }

  for (const target of config.targets ?? []) {
    if (lines.length > 0) {
      lines.push('');
    }
    lines.push(...serializeTarget(target));
  }

  return lines.length > 0 ? `${lines.join('\n')}\n` : '';
}

export function serializeBrokerConfigToml(config: BrokerConfigEditor): string {
  const lines: string[] = [];
  lines.push(`auto_approve_allowed = ${config.auto_approve_allowed ? 'true' : 'false'}`);

  lines.push('');
  lines.push('[whitelist]');
  lines.push(...writeStringArray('allowed', config.whitelist.allowed ?? []));
  lines.push(...writeStringArray('denied', config.whitelist.denied ?? []));
  lines.push(...writeInlineStringMap('arg_rules', config.whitelist.arg_rules ?? {}));

  lines.push('');
  lines.push('[limits]');
  lines.push(`timeout_secs = ${config.limits.timeout_secs}`);
  lines.push(`max_output_bytes = ${config.limits.max_output_bytes}`);

  return `${lines.join('\n')}\n`;
}
