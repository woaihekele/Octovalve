<template>
  <div class="chat-row" :class="[`chat-row--${message.role}`, { 'chat-row--streaming': isStreaming }]">
    <div class="chat-row__content">
      <!-- Thinking section (collapsible) -->
      <ReasoningBlock
        v-if="hasMeaningfulThinking"
        :show="thinkingOpen"
        :streaming="isStreaming"
        :preview-text="thinkingContent"
        :preview-active="!hasMeaningfulResponse"
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
        :ref="(el) => registerBubble?.(message.id, el as HTMLElement | null)"
        :style="bubbleStyle?.(message.id)"
      >
        <div v-if="message.role === 'assistant' && isStreaming && !responseContent && !thinkingContent" class="chat-row__thinking-card">
          <div class="chat-row__thinking-shimmer"></div>
          <div class="chat-row__thinking-icon-wrapper">
            <div class="chat-row__thinking-ping"></div>
            <div class="chat-row__thinking-icon-bg">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M12 2a4 4 0 0 1 4 4v2a4 4 0 0 1-8 0V6a4 4 0 0 1 4-4z"/>
                <path d="M16 14a4 4 0 0 1-8 0"/>
                <path d="M9 18h6"/>
                <path d="M10 22h4"/>
              </svg>
            </div>
          </div>
          <div class="chat-row__thinking-text">
            <span class="chat-row__thinking-title">AI 正在思考...</span>
            <span class="chat-row__thinking-subtitle">
              处理中
              <span class="chat-row__thinking-dots">
                <span></span><span></span><span></span>
              </span>
            </span>
          </div>
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
import { computed, ref, type CSSProperties } from 'vue';
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

const thinkingOpen = ref(false);

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
    }
    
    .chat-row__bubble {
      background: linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%);
      color: #fff;
      border-radius: 18px 18px 4px 18px;
      width: fit-content;
      max-width: 80%;
      box-shadow: 0 2px 8px rgba(99, 102, 241, 0.25);
    }
    
    .chat-row__text {
      color: #fff;
    }
  }

  &--assistant {
    justify-content: flex-start;
    
    .chat-row__content {
      align-items: flex-start;
    }
    
    .chat-row__bubble {
      background: rgb(var(--color-panel-muted));
      border-radius: 18px 18px 18px 4px;
      width: fit-content;
      max-width: 90%;
      box-shadow: 0 1px 3px rgba(0, 0, 0, 0.06);
      border: 1px solid rgba(0, 0, 0, 0.04);
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
  }

  &__thinking-card {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    position: relative;
    overflow: hidden;
    border-radius: 16px;
    background: white;
    border: 1px solid rgba(99, 102, 241, 0.15);
    box-shadow: 0 2px 12px rgba(99, 102, 241, 0.08);
  }

  &__thinking-shimmer {
    position: absolute;
    inset: 0;
    background: linear-gradient(
      90deg,
      transparent 0%,
      rgba(99, 102, 241, 0.06) 50%,
      transparent 100%
    );
    animation: shimmer 1.5s infinite;
    transform: translateX(-100%);
  }

  &__thinking-icon-wrapper {
    position: relative;
    z-index: 1;
  }

  &__thinking-ping {
    position: absolute;
    inset: 0;
    background: rgba(99, 102, 241, 0.4);
    border-radius: 50%;
    animation: ping 1.5s ease-out infinite;
  }

  &__thinking-icon-bg {
    position: relative;
    width: 36px;
    height: 36px;
    background: linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%);
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: white;
    box-shadow: 0 2px 8px rgba(99, 102, 241, 0.3);
    
    svg {
      animation: pulse-icon 2s ease-in-out infinite;
    }
  }

  &__thinking-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    z-index: 1;
  }

  &__thinking-title {
    font-size: 13px;
    font-weight: 600;
    color: #374151;
  }

  &__thinking-subtitle {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: #9ca3af;
  }

  &__thinking-dots {
    display: flex;
    gap: 2px;
    margin-left: 2px;

    span {
      width: 3px;
      height: 3px;
      background: #9ca3af;
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
      background: rgba(0, 0, 0, 0.2);
      padding: 12px 14px;
      border-radius: 8px;
      overflow-x: auto;
      margin: 10px 0;
      font-size: 13px;
      font-family: 'SF Mono', Monaco, Consolas, monospace;

      code {
        background: none;
        padding: 0;
        font-size: inherit;
      }
    }

    :deep(code) {
      background: rgba(0, 0, 0, 0.1);
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

@keyframes shimmer {
  0% {
    transform: translateX(-100%);
  }
  100% {
    transform: translateX(100%);
  }
}

@keyframes ping {
  0% {
    transform: scale(1);
    opacity: 0.6;
  }
  75%, 100% {
    transform: scale(1.8);
    opacity: 0;
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

@keyframes dot-bounce {
  0%, 60%, 100% {
    transform: translateY(0);
  }
  30% {
    transform: translateY(-3px);
  }
}
</style>
