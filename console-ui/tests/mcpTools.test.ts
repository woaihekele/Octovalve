import { describe, expect, it } from 'vitest';
import { buildMcpTools } from '../src/shared/mcpTools';

describe('mcpTools', () => {
  it('builds run_command and list_targets schemas', () => {
    const tools = buildMcpTools(['dev1', 'dev2'], 'dev1');
    const toolNames = tools.map((tool) => tool.function.name);
    expect(toolNames).toEqual(['run_command', 'list_targets']);

    const runCommand = tools[0];
    const params = runCommand.function.parameters as Record<string, unknown>;
    expect(params.required).toEqual(['command', 'intent', 'target']);
    const props = params.properties as Record<string, Record<string, unknown>>;
    expect(props.target).toMatchObject({
      enum: ['dev1', 'dev2'],
      default: 'dev1',
    });
  });
});
