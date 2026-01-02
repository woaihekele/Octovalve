<template>
  <div class="chat-row" :class="[`chat-row--${message.role}`, { 'chat-row--streaming': isStreaming }]">
    <div ref="rowRef" class="chat-row__content">
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
          <MarkdownRender
            :custom-id="`chat-thinking-${message.id}`"
            :content="thinkingContent"
            :is-dark="isDark"
            :max-live-nodes="isStreaming ? 0 : undefined"
            :batch-rendering="isStreaming"
            :render-batch-size="16"
            :render-batch-delay="8"
            :final="!isStreaming"
          />
        </template>
      </ReasoningBlock>
      <!-- Tool calls -->
      <div v-if="message.toolCalls && message.toolCalls.length > 0" class="chat-row__tools">
        <ToolCallCard v-for="tc in message.toolCalls" :key="tc.id" :tool-call="tc" />
      </div>
      <!-- Main response bubble -->
      <div
        class="chat-row__bubble"
        v-if="responseContent || message.role === 'user' || (!thinkingContent && !responseContent)"
        :ref="handleBubbleRef"
        :style="resolvedBubbleStyle"
      >
        <div v-if="message.role === 'assistant' && isStreaming && !responseContent && !thinkingContent" class="chat-row__thinking-inline">
          <span class="chat-row__thinking-inline-label">思考中</span>
          <span class="chat-row__thinking-dots" aria-hidden="true">
            <span></span><span></span><span></span>
          </span>
        </div>
        <div v-else class="chat-row__text">
          <template v-if="message.role === 'assistant'">
            <MarkdownRender
              :custom-id="`chat-${message.id}`"
              :content="assistantMarkdown"
              :is-dark="isDark"
              :custom-html-tags="['thinking']"
              :max-live-nodes="isStreaming ? 0 : undefined"
              :batch-rendering="isStreaming"
              :render-batch-size="16"
              :render-batch-delay="8"
              :final="!isStreaming"
            />
          </template>
          <template v-else>
            <MarkdownRender
              :custom-id="`chat-${message.id}`"
              :content="message.content"
              :is-dark="isDark"
            />
          </template>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch, nextTick, onBeforeUnmount, type ComponentPublicInstance, type CSSProperties } from 'vue';
import hljs from 'highlight.js/lib/common';
import MarkdownRender from 'markstream-vue';
import type { ChatMessage } from '../types';
import ToolCallCard from './ToolCallCard.vue';
import ReasoningBlock from './ReasoningBlock.vue';

interface Props {
  message: ChatMessage;
  isLast?: boolean;
  registerBubble?: (messageId: string, el: HTMLElement | null) => void;
  bubbleStyle?: (messageId: string) => CSSProperties | undefined;
}

const props = withDefaults(defineProps<Props>(), {
  isLast: false,
});

const emit = defineEmits<{
  (e: 'toggle-thinking', opened: boolean): void;
}>();

const rowRef = ref<HTMLElement | null>(null);
const thinkingOpen = ref(false);

const highlightToken = ref(0);
let highlightObserver: MutationObserver | null = null;

const highlightThrottleMs = 250;
let highlightLastAt = 0;
let highlightTimer: number | null = null;

function highlightCodeBlocks() {
  const root = rowRef.value;
  if (!root) {
    return;
  }
  const pres = root.querySelectorAll('pre');
  pres.forEach((pre) => {
    const preEl = pre as HTMLElement;
    let codeEl = preEl.querySelector('code') as HTMLElement | null;

    if (!codeEl) {
      codeEl = document.createElement('code');
      codeEl.textContent = preEl.textContent ?? '';
      preEl.textContent = '';
      preEl.appendChild(codeEl);
    }

    // In streaming mode, the code block content is changing. Re-highlight by
    // resetting to plain text before applying highlight.js again.
    if (isStreaming.value && codeEl.classList.contains('hljs')) {
      const raw = codeEl.textContent ?? '';
      codeEl.classList.remove('hljs');
      codeEl.removeAttribute('data-highlighted');
      codeEl.textContent = raw;
    } else if (codeEl.classList.contains('hljs')) {
      return;
    }
    try {
      hljs.highlightElement(codeEl);
    } catch {
      // ignore highlight errors
    }
  });
}

function requestHighlight({ force }: { force: boolean }) {
  if (typeof window === 'undefined') {
    void scheduleHighlight();
    return;
  }

  const now = window.Date.now();
  const elapsed = now - highlightLastAt;
  if (force || elapsed >= highlightThrottleMs) {
    highlightLastAt = now;
    if (highlightTimer !== null) {
      window.clearTimeout(highlightTimer);
      highlightTimer = null;
    }
    void scheduleHighlight();
    return;
  }

  if (highlightTimer !== null) {
    return;
  }

  highlightTimer = window.setTimeout(() => {
    highlightTimer = null;
    highlightLastAt = window.Date.now();
    void scheduleHighlight();
  }, Math.max(0, highlightThrottleMs - elapsed));
}

function shouldRetryHighlight(): boolean {
  const root = rowRef.value;
  if (!root) {
    return false;
  }
  const blocks = root.querySelectorAll('pre code');
  if (blocks.length === 0) {
    return true;
  }
  for (const block of Array.from(blocks)) {
    if (!(block as HTMLElement).classList.contains('hljs')) {
      return true;
    }
  }
  return false;
}

function runHighlightPass(token: number, attempt: number) {
  if (highlightToken.value !== token) {
    return;
  }
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      if (highlightToken.value !== token) {
        return;
      }
      highlightCodeBlocks();
      if (attempt >= 3) {
        return;
      }
      if (!shouldRetryHighlight()) {
        return;
      }
      window.setTimeout(() => {
        runHighlightPass(token, attempt + 1);
      }, 80);
    });
  });
}

async function scheduleHighlight() {
  const token = highlightToken.value + 1;
  highlightToken.value = token;
  await nextTick();
  runHighlightPass(token, 0);
}

function ensureHighlightObserver() {
  if (typeof MutationObserver === 'undefined') {
    return;
  }
  const root = rowRef.value;
  if (!root) {
    return;
  }
  if (highlightObserver) {
    return;
  }
  highlightObserver = new MutationObserver(() => {
    requestHighlight({ force: false });
  });
  highlightObserver.observe(root, { childList: true, subtree: true, characterData: true });
}

onBeforeUnmount(() => {
  if (highlightObserver) {
    highlightObserver.disconnect();
    highlightObserver = null;
  }
  if (typeof window !== 'undefined' && highlightTimer !== null) {
    window.clearTimeout(highlightTimer);
    highlightTimer = null;
  }
});

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
    return { thinking, response };
  }
  
  // Check for **Thinking** or **思考** headers
  const thinkingHeaderMatch = content.match(/^\*\*(?:Thinking|思考|Preparing)[^*]*\*\*\n?([\s\S]*?)(?=\n\n|$)/i);
  if (thinkingHeaderMatch) {
    const thinking = thinkingHeaderMatch[0].trim();
    const response = content.replace(thinkingHeaderMatch[0], '').trim();
    return { thinking, response };
  }
  
  return { thinking: '', response: content };
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

const smoothOptions = computed(() => ({
  minDelay: 24,
  chunkFactor: 9,
  maxCharsPerFrame: 64,
  streamEndStrategy: 'immediate' as const,
}));

function handleToggleThinking() {
  thinkingOpen.value = !thinkingOpen.value;
  emit('toggle-thinking', thinkingOpen.value);
  void scheduleHighlight();
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

const resolvedBubbleStyle = computed(() => {
  return props.bubbleStyle?.(props.message.id);
});

const handleBubbleRef = (el: Element | ComponentPublicInstance | null) => {
  if (props.message.role !== 'assistant') {
    return;
  }
  props.registerBubble?.(props.message.id, (el as HTMLElement | null) ?? null);
};

watch(
  () => [props.message.content, props.message.reasoning, thinkingOpen.value, isStreaming.value],
  () => {
    ensureHighlightObserver();
    requestHighlight({ force: false });
  },
  { immediate: true, flush: 'post' }
);

watch(
  () => isStreaming.value,
  (streaming) => {
    if (!streaming) {
      requestHighlight({ force: true });
    }
  }
);
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
      padding-top: 9px;
      padding-bottom: 7px;
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
      line-height: 1;
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
    }
    
    .chat-row__bubble {
      display: inline-block;
      width: fit-content;
      max-width: 100%;
      background: transparent;
      border: none;
      border-radius: 0;
      box-shadow: none;
    }
  }

  &__tools {
    margin-bottom: 8px;
    width: fit-content;
    max-width: 90%;
  }

  &__content {
    display: flex;
    flex-direction: column;
    max-width: 90%;
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
      &:last-child { margin-bottom: 0; }
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

      &:nth-child(2) { animation-delay: 0.15s; }
      &:nth-child(3) { animation-delay: 0.3s; }
    }
  }

  &__text {
    font-size: 14px;
    line-height: 1.6;
    color: rgb(var(--color-text));
    word-break: break-word;
    overflow-wrap: anywhere;

    // Markdown elements
    :deep(p) {
      margin: 0 0 0.75em 0;
      &:last-child { margin-bottom: 0; }
    }

    :deep(ul), :deep(ol) {
      margin: 0.5em 0;
      padding-left: 1.5em;
    }

    :deep(li) {
      margin: 0.25em 0;
    }

    :deep(h1), :deep(h2), :deep(h3), :deep(h4) {
      margin: 1em 0 0.5em 0;
      font-weight: 600;
      &:first-child { margin-top: 0; }
    }

    :deep(h1) { font-size: 1.4em; }
    :deep(h2) { font-size: 1.2em; }
    :deep(h3) { font-size: 1.1em; }

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
      color: #8b5cf6;
      text-decoration: none;
      &:hover { text-decoration: underline; }
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
  to { transform: rotate(360deg); }
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
