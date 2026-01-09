import type { ChatMessageBlock } from '../types';

export type IdGenerator = () => string;

export function concatAcpTextChunk(existing: string, chunk: string): string {
  if (!chunk) {
    return existing;
  }
  if (!existing) {
    return chunk;
  }

  const lastChar = existing[existing.length - 1] ?? '';
  const firstChar = chunk[0] ?? '';
  if (/\s/.test(lastChar) || /\s/.test(firstChar)) {
    return existing + chunk;
  }

  if (
    (existing.endsWith('**') && chunk.startsWith('**')) ||
    (existing.endsWith('__') && chunk.startsWith('__'))
  ) {
    return `${existing}\n${chunk}`;
  }

  return existing + chunk;
}

export function appendReasoningBlock(
  blocks: ChatMessageBlock[] | undefined,
  delta: string,
  generateId: IdGenerator
): { blocks: ChatMessageBlock[]; startedNewBlock: boolean } {
  const current = Array.isArray(blocks) ? blocks : [];
  const next = current.slice();
  const last = next[next.length - 1];
  if (last && last.type === 'reasoning') {
    next[next.length - 1] = { ...last, content: concatAcpTextChunk(last.content, delta) };
    return { blocks: next, startedNewBlock: false };
  }
  next.push({ type: 'reasoning', id: generateId(), content: delta });
  return { blocks: next, startedNewBlock: true };
}

export function ensureToolCallBlock(
  blocks: ChatMessageBlock[] | undefined,
  toolCallId: string
): { blocks: ChatMessageBlock[]; inserted: boolean } {
  const current = Array.isArray(blocks) ? blocks : [];
  const exists = current.some((b) => b.type === 'tool_call' && b.toolCallId === toolCallId);
  if (exists) {
    return { blocks: current, inserted: false };
  }
  return { blocks: [...current, { type: 'tool_call', toolCallId }], inserted: true };
}

