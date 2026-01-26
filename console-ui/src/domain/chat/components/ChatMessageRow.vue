<template>
  <div class="chat-row" :class="[`chat-row--${message.role}`, { 'chat-row--streaming': isStreaming }]">
    <div ref="rowRef" class="chat-row__content">
      <template v-if="hasTimelineBlocks">
        <template v-for="block in timelineBlocks" :key="block.type === 'reasoning' ? block.id : block.toolCallId">
          <ReasoningBlock
            v-if="block.type === 'reasoning' && hasMeaningfulText(block.content)"
            :show="isBlockOpen(block.id)"
            :streaming="isBlockStreaming(block.id)"
            :preview-text="block.content"
            :preview-active="!hasMeaningfulResponse"
            :preview-custom-id="`chat-thinking-preview-${message.id}-${block.id}`"
            :is-dark="isDark"
            :smooth-options="smoothOptions"
            @toggle="handleToggleThinkingBlock(block.id)"
          >
            <template #body>
              <ChatMarkdown
                :text="block.content"
                :streaming="isBlockStreaming(block.id)"
                :content-key="`chat-thinking-${message.id}-${block.id}`"
                :smooth-options="smoothOptions"
              />
            </template>
          </ReasoningBlock>
          <ToolCallCard
            v-else-if="block.type === 'tool_call' && toolCallById.get(block.toolCallId)"
            :tool-call="toolCallById.get(block.toolCallId)!"
          />
        </template>
      </template>
      <template v-else>
        <!-- Thinking section (collapsible) -->
        <ReasoningBlock
          v-if="hasMeaningfulThinking"
          :show="thinkingOpen"
          :streaming="isStreaming"
          :preview-text="thinkingContent"
          :preview-active="!hasMeaningfulResponse"
          :preview-custom-id="`chat-thinking-preview-${message.id}`"
          :is-dark="isDark"
          :smooth-options="smoothOptions"
          @toggle="handleToggleThinking"
        >
          <template #body>
            <ChatMarkdown
              :text="thinkingContent"
              :streaming="isStreaming"
              :content-key="`chat-thinking-${message.id}`"
              :smooth-options="smoothOptions"
            />
          </template>
        </ReasoningBlock>
        <!-- Tool calls -->
        <div v-if="message.toolCalls && message.toolCalls.length > 0" class="chat-row__tools">
          <ToolCallCard v-for="tc in message.toolCalls" :key="tc.id" :tool-call="tc"/>
        </div>
      </template>
      <!-- Main response bubble -->
      <div
        class="chat-row__bubble"
        v-if="responseContent || message.role === 'user' || (!thinkingContent && !responseContent)"
      >
        <div v-if="message.role === 'assistant' && isStreaming && !responseContent && !thinkingContent" class="chat-row__thinking-inline">
          <span class="chat-row__thinking-inline-label">{{ $t('chat.thinking') }}</span>
          <span class="chat-row__thinking-dots" aria-hidden="true">
            <span></span><span></span><span></span>
          </span>
        </div>
        <div v-else class="chat-row__text">
          <template v-if="message.role === 'assistant'">
            <ChatMarkdown
              :text="assistantMarkdown"
              :streaming="isStreaming"
              :content-key="`chat-${message.id}`"
              :smooth-options="smoothOptions"
            />
          </template>
          <template v-else>
            <ChatMarkdown :text="message.content" :streaming="false" :content-key="`chat-${message.id}`"/>
          </template>
        </div>
        <div v-if="message.images?.length" class="chat-row__images">
          <img
            v-for="(img, index) in message.images"
            :key="`${message.id}-img-${index}`"
            :src="img"
            class="chat-row__image"
            alt="image"
          />
        </div>
        <div v-if="message.files?.length" class="chat-row__files">
          <div v-for="(file, index) in message.files" :key="`${message.id}-file-${index}`" class="chat-row__file">
            {{ file }}
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import {computed, ref} from 'vue';
import type { ChatMessage, ToolCall } from '../types';
import ToolCallCard from './ToolCallCard.vue';
import ReasoningBlock from './ReasoningBlock.vue';
import ChatMarkdown from './ChatMarkdown.vue';

interface Props {
  message: ChatMessage;
  isLast?: boolean;
}

const props = withDefaults(defineProps<Props>(), {
  isLast: false,
});

const emit = defineEmits<{
  (e: 'toggle-thinking', opened: boolean): void;
}>();

const rowRef = ref<HTMLElement | null>(null);
const thinkingOpen = ref(false);
const reasoningBlocksOpen = ref<Record<string, boolean>>({});

const isStreaming = computed(() => {
  return props.message.status === 'streaming' && props.isLast;
});

const isDark = computed(() => {
  if (typeof document === 'undefined') {
    return true;
  }
  return document.documentElement.dataset.theme !== 'light';
});

// Parse thinking content from <thinking> tags or reasoning_content
const parsedContent = computed(() => {
  const content = props.message.content || '';

  // Check for <thinking> tags
  const thinkingMatch = content.match(/<thinking>([\s\S]*?)<\/thinking>/);
  if (thinkingMatch) {
    const thinking = thinkingMatch[1].trim();
    const response = content.replace(/<thinking>[\s\S]*?<\/thinking>/, '').trim();
    return {thinking, response};
  }

  // Check for **Thinking** or **思考** headers
  const thinkingHeaderMatch = content.match(/^\*\*(?:Thinking|思考|Preparing)[^*]*\*\*\n?([\s\S]*?)(?=\n\n|$)/i);
  if (thinkingHeaderMatch) {
    const thinking = thinkingHeaderMatch[0].trim();
    const response = content.replace(thinkingHeaderMatch[0], '').trim();
    return {thinking, response};
  }

  return {thinking: '', response: content};
});

const thinkingContent = computed(() => {
  const direct = props.message.reasoning;
  if (direct && direct.trim()) {
    return direct;
  }
  return parsedContent.value.thinking;
});
const responseContent = computed(() => parsedContent.value.response);

const hasMeaningfulText = (text?: string | null): boolean => {
  if (text === undefined || text === null) {
    return false;
  }
  const cleaned = text
    .replace(/<[^>]*?>/g, '')
    .replace(/[\s\u200B\u00A0]+/g, '');
  return cleaned !== '';
};

const hasMeaningfulThinking = computed(() => hasMeaningfulText(thinkingContent.value));
const hasMeaningfulResponse = computed(() => hasMeaningfulText(responseContent.value));

const hasTimelineBlocks = computed(() => {
  const blocks = props.message.blocks;
  return Array.isArray(blocks) && blocks.length > 0;
});

const timelineBlocks = computed(() => props.message.blocks || []);

const toolCallById = computed(() => {
  const map = new Map<string, ToolCall>();
  for (const tc of props.message.toolCalls || []) {
    map.set(tc.id, tc);
  }
  return map;
});

const lastReasoningBlockId = computed(() => {
  const blocks = props.message.blocks;
  if (!Array.isArray(blocks) || blocks.length === 0) {
    return null;
  }
  for (let i = blocks.length - 1; i >= 0; i -= 1) {
    const block = blocks[i];
    if (block?.type === 'reasoning') {
      return block.id;
    }
  }
  return null;
});

const smoothOptions = computed(() => ({
  minDelay: 24,
  chunkFactor: 9,
  maxCharsPerFrame: 64,
  streamEndStrategy: 'immediate' as const,
}));

function handleToggleThinking() {
  thinkingOpen.value = !thinkingOpen.value;
  emit('toggle-thinking', thinkingOpen.value);
}

function isBlockOpen(blockId: string): boolean {
  return reasoningBlocksOpen.value[blockId] ?? false;
}

function isBlockStreaming(blockId: string): boolean {
  return isStreaming.value && lastReasoningBlockId.value === blockId;
}

function handleToggleThinkingBlock(blockId: string) {
  const next = !isBlockOpen(blockId);
  reasoningBlocksOpen.value = { ...reasoningBlocksOpen.value, [blockId]: next };
  emit('toggle-thinking', next);
}

const assistantMarkdown = computed(() => {
  const response = responseContent.value;
  const cursor = isStreaming.value ? '\n\n<span class="chat-row__cursor"></span>' : '';
  const base = response || '';
  if (!base && isStreaming.value) {
    return '<span class="chat-row__cursor"></span>';
  }
  return `${base}${cursor}`;
});

</script>

<style scoped lang="scss">
.chat-row {
  display: flex;
  padding: 8px 16px;
  animation: slideIn 0.25s ease-out;

  &--user {
    justify-content: flex-end;

    .chat-row__content {
      align-items: flex-end;
      width: 100%;
      max-width: 100%;
    }

    .chat-row__bubble {
      background: rgb(var(--color-panel-muted));
      border: 1px solid rgb(var(--color-border));
      color: rgb(var(--color-text));
      border-radius: 18px 18px 4px 18px;
      display: inline-block;
      width: auto;
      padding-top: 8px;
      padding-bottom: 8px;
      max-width: 80%;
      box-shadow: none;
    }

    .chat-row__text {
      color: rgb(var(--color-text));
      display: inline-block;
      min-width: 0;
      max-width: 100%;
      font-size: 14px;
      white-space: pre-wrap;
      line-height: 1.5;
      word-break: keep-all;
      overflow-wrap: break-word;

      :deep(> *) {
        display: inline-block;
        width: max-content;
        max-width: 100%;
        line-height: inherit;
        margin: 0;
        padding: 0;
        word-break: inherit;
        overflow-wrap: inherit;
      }

      :deep(*) {
        line-height: inherit;
      }

      :deep(p) {
        display: inline;
        margin: 0;
      }

      :deep(pre) {
        max-width: 100%;
        overflow-x: auto;
      }

      :deep(table) {
        display: block;
        max-width: 100%;
        overflow-x: auto;
      }

      :deep(img) {
        max-width: 100%;
        height: auto;
      }
    }
  }

  &--assistant {
    justify-content: center;
    padding-left: 0;
    padding-right: 0;

    .chat-row__content {
      align-items: stretch;
      max-width: 100%;
      width: 100%;
      padding: 0 14px;
      min-width: 0;
    }

    .chat-row__bubble {
      display: block;
      width: 100%;
      min-width: 0;
      max-width: 100%;
      padding: 0;
      overflow: visible;
      background: transparent;
      border: none;
      border-radius: 0;
      box-shadow: none;
    }
  }

  &__tools {
    margin-bottom: 8px;
    width: 100%;
    max-width: 100%;
  }

  &__content {
    display: flex;
    flex-direction: column;
    max-width: 90%;
    min-width: 0;
  }

  &__thinking {
    margin-bottom: 8px;
    border-radius: 12px;
    background: rgba(139, 92, 246, 0.08);
    overflow: hidden;

    &--open {
      .chat-row__thinking-toggle {
        border-bottom: 1px solid rgba(139, 92, 246, 0.15);
      }
    }
  }

  &__thinking-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 8px 12px;
    border: none;
    background: none;
    cursor: pointer;
    font-size: 12px;
    color: rgb(var(--color-text-muted));
    transition: all 0.2s ease;

    &:hover {
      background: rgba(139, 92, 246, 0.1);
    }
  }

  &__thinking-icon {
    font-size: 10px;
    color: #8b5cf6;
  }

  &__thinking-label {
    flex: 1;
    text-align: left;
  }

  &__thinking-spinner {
    width: 10px;
    height: 10px;
    border: 2px solid rgba(139, 92, 246, 0.3);
    border-top-color: #8b5cf6;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  &__thinking-content {
    padding: 10px 12px;
    font-size: 13px;
    line-height: 1.5;
    color: rgb(var(--color-text-muted));

    :deep(p) {
      margin: 0 0 0.5em 0;

      &:last-child {
        margin-bottom: 0;
      }
    }
  }

  &__bubble {
    padding: 10px 14px;
    transition: all 0.2s ease;
    overflow: hidden;
  }

  &__thinking-inline {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: rgb(var(--color-text-muted));
  }

  &__thinking-inline-label {
    font-weight: 500;
  }

  &__thinking-dots {
    display: flex;
    gap: 2px;
    margin-left: 2px;

    span {
      width: 3px;
      height: 3px;
      background: rgb(var(--color-text-muted));
      border-radius: 50%;
      animation: dot-bounce 1.4s ease-in-out infinite;

      &:nth-child(2) {
        animation-delay: 0.15s;
      }

      &:nth-child(3) {
        animation-delay: 0.3s;
      }
    }
  }

  &__text {
    font-size: 14px;
    line-height: 1.6;
    color: rgb(var(--color-text));
    word-break: break-word;
    overflow-wrap: anywhere;
    min-width: 0;
    --chat-markdown-list-indent: 0.8em;

    // Markdown elements
    :deep(p) {
      margin: 0 0 0.75em 0;

      &:last-child {
        margin-bottom: 0;
      }
    }

    :deep(ul), :deep(ol) {
      margin: 0.5em 0;
      padding-left: var(--chat-markdown-list-indent);
    }

    /* Tailwind preflight 会把 ul/ol 的 list-style 重置为 none，需要在消息内容里显式恢复 */
    :deep(ul) {
      list-style-type: disc;
    }

    :deep(ol) {
      list-style-type: decimal;
    }

    :deep(li) {
      margin: 0.25em 0;
      /* 气泡容器使用 overflow:hidden 时，默认 list marker（outside）可能被裁剪，导致 1/2/3 或圆点看不到 */
      list-style-position: inside;
    }

    :deep(h1), :deep(h2), :deep(h3), :deep(h4) {
      margin: 1em 0 0.5em 0;
      font-weight: 600;

      &:first-child {
        margin-top: 0;
      }
    }

    :deep(h1) {
      font-size: 1.4em;
    }

    :deep(h2) {
      font-size: 1.2em;
    }

    :deep(h3) {
      font-size: 1.1em;
    }

    :deep(blockquote) {
      margin: 0.5em 0;
      padding-left: 1em;
      border-left: 3px solid rgb(var(--color-border));
      color: rgb(var(--color-text-muted));
    }

    :deep(pre) {
      background: rgb(var(--color-panel-muted));
      border: 1px solid rgb(var(--color-border));
      padding: 12px 14px;
      border-radius: 8px;
      overflow-x: auto;
      margin: 10px 0;
      font-size: 13px;
      font-family: 'SF Mono', Monaco, Consolas, monospace;
      max-width: 100%;
      box-sizing: border-box;

      code {
        background: none;
        border: none;
        padding: 0;
        font-size: inherit;
      }
    }

    :deep(code) {
      background: rgb(var(--color-panel-muted));
      border: 1px solid rgb(var(--color-border));
      padding: 2px 6px;
      border-radius: 4px;
      font-size: 13px;
      font-family: 'SF Mono', Monaco, Consolas, monospace;
    }

    :deep(a) {
      color: rgb(var(--color-accent));
      text-decoration: none;

      &:hover {
        text-decoration: underline;
      }
    }

    :deep(hr) {
      border: none;
      border-top: 1px solid rgb(var(--color-border));
      margin: 1em 0;
    }

    :deep(.chat-row__cursor) {
      display: inline-block;
      width: 3px;
      height: 1.1em;
      background: linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%);
      margin-left: 2px;
      vertical-align: text-bottom;
      border-radius: 1px;
      animation: cursor-blink 0.8s ease-in-out infinite;
    }
  }

  &__images {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 8px;
  }

  &__image {
    max-width: 220px;
    max-height: 180px;
    border-radius: 8px;
    border: 1px solid rgb(var(--color-border));
    object-fit: cover;
  }

  &__files {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }

  &__file {
    padding: 4px 8px;
    border-radius: 999px;
    border: 1px solid rgb(var(--color-border));
    background: rgb(var(--color-panel-muted));
    color: rgb(var(--color-text));
    font-size: 12px;
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  // Smooth text appearance for streaming
  &--streaming {
    .chat-row__text {
      :deep(p:last-child),
      :deep(li:last-child),
      :deep(code:last-child) {
        animation: text-appear 0.1s ease-out;
      }
    }
  }
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

@keyframes typing {
  0%, 60%, 100% {
    transform: translateY(0);
    opacity: 0.4;
  }
  30% {
    transform: translateY(-6px);
    opacity: 1;
  }
}

@keyframes cursor-blink {
  0%, 100% {
    opacity: 1;
    transform: scaleY(1);
  }
  50% {
    opacity: 0.3;
    transform: scaleY(0.8);
  }
}

@keyframes text-appear {
  from {
    opacity: 0.7;
  }
  to {
    opacity: 1;
  }
}

@keyframes slideIn {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes dot-bounce {
  0%, 80%, 100% {
    transform: translateY(0);
    opacity: 0.5;
  }
  40% {
    transform: translateY(-3px);
    opacity: 1;
  }
}

@keyframes pulse-icon {
  0%, 100% {
    transform: scale(1);
  }
  50% {
    transform: scale(1.1);
  }
}
</style>
