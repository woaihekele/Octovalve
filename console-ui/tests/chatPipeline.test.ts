import { describe, expect, it } from 'vitest';
import type { ImageAttachment, TextAttachment } from '../src/domain/chat/types';
import {
  buildPromptBlocks,
  normalizeSendOptions,
  parseAcpHistory,
  toAcpPromptBlocks,
  toOpenAiContentParts,
  toPlainText,
} from '../src/domain/chat/pipeline/chatPipeline';

describe('chatPipeline', () => {
  it('normalizes send options from string', () => {
    expect(normalizeSendOptions('hi')).toEqual({ content: 'hi' });
  });

  it('builds prompt blocks from content, images, and files', () => {
    const images: ImageAttachment[] = [
      { kind: 'image', data: 'abc', mimeType: 'image/png', previewUrl: 'blob:img' },
    ];
    const files: TextAttachment[] = [
      { kind: 'text', name: 'note.txt', mimeType: 'text/plain', content: 'hello' },
    ];
    const blocks = buildPromptBlocks({ content: 'hello', images, files });
    expect(blocks).toEqual([
      { type: 'text', text: 'hello' },
      { type: 'image', data: 'abc', mimeType: 'image/png', previewUrl: 'blob:img' },
      { type: 'text', text: '[File: note.txt]\nhello' },
    ]);
  });

  it('converts blocks to ACP/OpenAI payloads', () => {
    const blocks = buildPromptBlocks({
      content: 'hello',
      images: [{ kind: 'image', data: 'abc', mimeType: 'image/png', previewUrl: 'blob:img' }],
    });
    const acp = toAcpPromptBlocks(blocks);
    expect(acp).toEqual([
      { type: 'text', text: 'hello' },
      { type: 'image', data: 'abc', mime_type: 'image/png' },
    ]);

    const openai = toOpenAiContentParts(blocks);
    expect(openai).toEqual([
      { type: 'text', text: 'hello' },
      { type: 'image_url', image_url: { url: 'blob:img' } },
    ]);
  });

  it('parses ACP history with fallback fields', () => {
    let id = 0;
    const history = {
      items: [
        { role: 'user', content: 'hi', ts: 10 },
        { speaker: 'assistant', message: ['hello', ' world'] },
        { type: 'system', value: 'sys', created_at: 42 },
      ],
    };
    const parsed = parseAcpHistory(history, () => `id-${id++}`, 1000);
    expect(parsed).toHaveLength(3);
    expect(parsed[0].id).toBe('id-0');
    expect(parsed[0].role).toBe('user');
    expect(parsed[0].content).toBe('hi');
    expect(parsed[0].ts).toBe(10);
    expect(parsed[1].content).toBe('hello world');
    expect(parsed[2].role).toBe('system');
  });

  it('extracts plain text from nested content', () => {
    expect(toPlainText({ content: [{ text: 'hi' }, { text: ' there' }] })).toBe('hi there');
  });
});
