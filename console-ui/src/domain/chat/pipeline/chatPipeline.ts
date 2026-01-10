import type {
  ChatMessage,
  ImageAttachment,
  PromptBlock,
  SendMessageOptions,
  TextAttachment,
} from '../types';
import type { AcpContentBlock } from '../services/acpService';
import type { OpenAiContentPart } from '../services/openaiService';

export function toPlainText(value: unknown): string {
  if (value === null || value === undefined) return '';
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  if (Array.isArray(value)) return value.map(toPlainText).filter(Boolean).join('');
  if (typeof value === 'object') {
    const obj = value as Record<string, unknown>;
    if (typeof obj.text === 'string') return obj.text;
    if (typeof obj.content === 'string') return obj.content;
    if (typeof obj.message === 'string') return obj.message;
    if (typeof obj.value === 'string') return obj.value;
    if (Array.isArray(obj.content)) return toPlainText(obj.content);
    if (Array.isArray(obj.prompt)) return toPlainText(obj.prompt);
    if (Array.isArray(obj.messages)) return toPlainText(obj.messages);
    if (Array.isArray(obj.blocks)) return toPlainText(obj.blocks);
  }
  return '';
}

export function toRole(value: unknown): 'user' | 'assistant' | 'system' {
  if (!value || typeof value !== 'string') return 'assistant';
  const v = value.toLowerCase();
  if (v === 'user') return 'user';
  if (v === 'assistant') return 'assistant';
  if (v === 'system') return 'system';
  return 'assistant';
}

export function toTimestamp(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  return null;
}

export function parseAcpHistory(
  history: unknown,
  generateId: () => string,
  now: number = Date.now()
): ChatMessage[] {
  if (!history) return [];

  const list = Array.isArray(history)
    ? history
    : typeof history === 'object' && history !== null && Array.isArray((history as any).items)
      ? ((history as any).items as unknown[])
      : typeof history === 'object' && history !== null && Array.isArray((history as any).history)
        ? ((history as any).history as unknown[])
        : [];

  if (!Array.isArray(list) || list.length === 0) return [];

  const parsed: ChatMessage[] = [];

  for (let i = 0; i < list.length; i += 1) {
    const item = list[i];
    const obj = typeof item === 'object' && item !== null ? (item as Record<string, unknown>) : null;
    const role = toRole(obj?.role ?? obj?.speaker ?? obj?.type);

    const content = toPlainText(
      obj?.content ??
        obj?.text ??
        obj?.message ??
        obj?.value ??
        obj?.prompt ??
        obj?.output ??
        obj?.response
    );

    if (!content.trim()) {
      continue;
    }

    const ts =
      toTimestamp(obj?.ts) ??
      toTimestamp(obj?.timestamp) ??
      toTimestamp(obj?.createdAt) ??
      toTimestamp(obj?.created_at) ??
      (now - (list.length - i) * 1000);

    parsed.push({
      id: generateId(),
      ts,
      type: 'say',
      say: 'text',
      role,
      content,
      status: 'complete',
      partial: false,
    });
  }

  return parsed;
}

export function normalizeSendOptions(input: string | SendMessageOptions): SendMessageOptions {
  if (typeof input === 'string') {
    return { content: input };
  }
  return {
    content: input.content ?? '',
    images: input.images ?? [],
    blocks: input.blocks,
    files: input.files ?? [],
    context: input.context,
  };
}

export function buildPromptBlocks(options: SendMessageOptions): PromptBlock[] {
  if (options.blocks && options.blocks.length > 0) {
    return options.blocks;
  }
  const blocks: PromptBlock[] = [];
  const text = options.content?.trim();
  if (text) {
    blocks.push({ type: 'text', text });
  }
  if (options.images) {
    for (const image of options.images) {
      blocks.push({
        type: 'image',
        data: image.data,
        mimeType: image.mimeType,
        previewUrl: image.previewUrl,
      });
    }
  }
  if (options.files) {
    for (const file of options.files) {
      blocks.push({
        type: 'text',
        text: `[File: ${file.name}]\n${file.content}`,
      });
    }
  }
  return blocks;
}

export function toAcpPromptBlocks(blocks: PromptBlock[]): AcpContentBlock[] {
  return blocks
    .map((block) => {
      if (block.type === 'text') {
        return { type: 'text', text: block.text } as const;
      }
      if (block.type === 'image') {
        return {
          type: 'image',
          data: block.data,
          mime_type: block.mimeType,
        } as const;
      }
      return null;
    })
    .filter((block): block is AcpContentBlock => block !== null);
}

export function toOpenAiContentParts(blocks: PromptBlock[]): OpenAiContentPart[] {
  return blocks
    .map((block) => {
      if (block.type === 'text') {
        return { type: 'text', text: block.text } as const;
      }
      if (block.type === 'image') {
        const url = block.previewUrl ?? `data:${block.mimeType};base64,${block.data}`;
        return { type: 'image_url', image_url: { url } } as const;
      }
      return null;
    })
    .filter((part): part is OpenAiContentPart => part !== null);
}

export function toDisplayImages(images?: ImageAttachment[]): string[] | undefined {
  if (!images || images.length === 0) {
    return undefined;
  }
  return images.map((img) => img.previewUrl);
}

export function toDisplayFiles(files?: TextAttachment[]): string[] | undefined {
  if (!files || files.length === 0) {
    return undefined;
  }
  return files.map((file) => file.name);
}

export function buildTextPrompt(options: SendMessageOptions): string {
  const parts: string[] = [];
  const text = options.content?.trim();
  if (text) {
    parts.push(text);
  }
  if (options.files) {
    for (const file of options.files) {
      parts.push(`[File: ${file.name}]\n${file.content}`);
    }
  }
  return parts.join('\n\n');
}
