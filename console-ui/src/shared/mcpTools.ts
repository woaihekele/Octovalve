import type { Tool } from '../domain/chat/services/openaiService';

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
        description: 'Forward command execution to a remote broker with manual approval.',
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
              enum: ['shell', 'argv'],
              default: 'shell',
              description: 'Execution mode: shell uses /bin/bash -lc, argv uses parsed pipeline.',
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
