import type { Tool } from '../domain/chat/services/openaiService';

export type McpToolInfo = {
  server: string;
  name: string;
  description?: string | null;
  inputSchema: Record<string, unknown> | null;
};

type McpToolMapping = Map<string, { server: string; name: string }>;

function normalizeIdentifier(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) {
    return 'tool';
  }
  const normalized = trimmed.replace(/[^a-zA-Z0-9_]/g, '_').replace(/_+/g, '_');
  return normalized.replace(/^_+|_+$/g, '') || 'tool';
}

function hashString(value: string): string {
  let hash = 2166136261;
  for (let i = 0; i < value.length; i += 1) {
    hash ^= value.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(36);
}

function shortenName(base: string, suffix: string): string {
  const maxBaseLen = Math.max(1, 63 - suffix.length);
  const trimmedBase = base.slice(0, maxBaseLen);
  return `${trimmedBase}_${suffix}`;
}

function buildOpenAiToolName(base: string, seed: string, used: Set<string>): string {
  let name = base;
  if (name.length > 64) {
    name = shortenName(base, hashString(seed));
  }
  if (!used.has(name)) {
    return name;
  }
  let attempt = 1;
  let candidate = name;
  while (used.has(candidate)) {
    candidate = shortenName(base, hashString(`${seed}:${attempt}`));
    attempt += 1;
  }
  return candidate;
}

function coerceSchema(schema: Record<string, unknown> | null): Record<string, unknown> {
  if (!schema || typeof schema !== 'object') {
    return { type: 'object', properties: {} };
  }
  return schema;
}

export function buildOpenAiToolsFromMcp(toolList: McpToolInfo[]): {
  tools: Tool[];
  mapping: McpToolMapping;
} {
  const tools: Tool[] = [];
  const mapping: McpToolMapping = new Map();
  const used = new Set<string>();
  const nameCounts = new Map<string, number>();

  for (const tool of toolList) {
    if (!tool || !tool.name) {
      continue;
    }
    const normalized = normalizeIdentifier(tool.name);
    nameCounts.set(normalized, (nameCounts.get(normalized) ?? 0) + 1);
  }

  for (const tool of toolList) {
    if (!tool || !tool.name || !tool.server) {
      continue;
    }
    const normalizedTool = normalizeIdentifier(tool.name);
    const normalizedServer = normalizeIdentifier(tool.server);
    const needsPrefix = (nameCounts.get(normalizedTool) ?? 0) > 1;
    const base = needsPrefix
      ? `mcp_${normalizedServer}_${normalizedTool}`
      : normalizedTool;
    const seed = `${tool.server}:${tool.name}`;
    const openaiName = buildOpenAiToolName(base, seed, used);
    used.add(openaiName);
    mapping.set(openaiName, { server: tool.server, name: tool.name });
    tools.push({
      type: 'function',
      function: {
        name: openaiName,
        description: typeof tool.description === 'string' ? tool.description : '',
        parameters: coerceSchema(tool.inputSchema),
      },
    });
  }

  return { tools, mapping };
}

export function buildMcpTools(targets: string[], defaultTarget?: string): Tool[] {
  const targetProperty: Record<string, unknown> = {
    type: 'string',
    description: 'Target name defined in octovalve-proxy config.',
  };
  if (targets.length > 0) {
    targetProperty.enum = targets;
  }
  if (defaultTarget) {
    targetProperty.default = defaultTarget;
  }

  return [
    {
      type: 'function',
      function: {
        name: 'run_command',
        description: 'Forward command execution to the console executor with manual approval.',
        parameters: {
          type: 'object',
          properties: {
            command: {
              type: 'string',
              description: 'Shell-like command line. Default mode executes via /bin/bash -lc.',
            },
            target: targetProperty,
            intent: {
              type: 'string',
              description: 'Why this command is needed (required for audit).',
            },
            mode: {
              type: 'string',
              enum: ['shell'],
              default: 'shell',
              description: 'Execution mode: shell uses /bin/bash -lc.',
            },
            cwd: {
              type: 'string',
              description: 'Working directory for the command.',
            },
            timeout_ms: {
              type: 'integer',
              minimum: 0,
              description: 'Override command timeout in milliseconds.',
            },
            max_output_bytes: {
              type: 'integer',
              minimum: 0,
              description: 'Override output size limit in bytes.',
            },
            env: {
              type: 'object',
              additionalProperties: { type: 'string' },
              description: 'Extra environment variables.',
            },
          },
          required: ['command', 'intent', 'target'],
        },
      },
    },
    {
      type: 'function',
      function: {
        name: 'list_targets',
        description: 'List available targets configured in octovalve-proxy.',
        parameters: {
          type: 'object',
          properties: {},
        },
      },
    },
  ];
}
