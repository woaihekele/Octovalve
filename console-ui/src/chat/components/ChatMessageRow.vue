<template>
  <div class="chat-row" :class="[`chat-row--${message.role}`, { 'chat-row--streaming': isStreaming }]">
    <div class="chat-row__content">
      <div v-if="message.role === 'assistant' && isStreaming" class="chat-row__indicator">
        <span class="chat-row__spinner"></span>
      </div>
      <div class="chat-row__body">
        <div class="chat-row__text" v-html="renderedContent"></div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue';
import type { ChatMessage } from '../types';

interface Props {
  message: ChatMessage;
  isLast?: boolean;
}

const props = withDefaults(defineProps<Props>(), {
  isLast: false,
});

const isStreaming = computed(() => {
  return props.message.status === 'streaming' && props.isLast;
});

const renderedContent = computed(() => {
  let content = props.message.content || '';
  if (!content && isStreaming.value) {
    return '<span class="chat-row__cursor"></span>';
  }
  
  // Escape HTML
  content = content.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  
  // Code blocks
  content = content.replace(/```(\w*)\n([\s\S]*?)```/g, (_, lang, code) => {
    return `<pre class="chat-row__code-block"><code class="language-${lang}">${code}</code></pre>`;
  });
  
  // Inline code
  content = content.replace(/`([^`]+)`/g, '<code class="chat-row__inline-code">$1</code>');
  
  // Bold
  content = content.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  
  // Italic
  content = content.replace(/\*([^*]+)\*/g, '<em>$1</em>');
  
  // Line breaks
  content = content.replace(/\n/g, '<br>');
  
  // Add cursor at end if streaming
  if (isStreaming.value) {
    content += '<span class="chat-row__cursor"></span>';
  }
  
  return content;
});
</script>

<style scoped lang="scss">
.chat-row {
  padding: 10px 15px 10px 15px;
  position: relative;

  &--user {
    .chat-row__content {
      background: rgb(var(--color-panel-muted));
      border-radius: 12px 12px 4px 12px;
      margin-left: 20px;
    }
  }

  &--assistant {
    .chat-row__content {
      margin-right: 20px;
    }
  }

  &__content {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }

  &__indicator {
    flex-shrink: 0;
    width: 16px;
    height: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    margin-top: 2px;
  }

  &__spinner {
    width: 12px;
    height: 12px;
    border: 2px solid rgb(var(--color-border));
    border-top-color: rgb(var(--color-accent));
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  &__body {
    flex: 1;
    min-width: 0;
  }

  &__text {
    font-size: 13px;
    line-height: 1.5;
    color: rgb(var(--color-text));
    word-break: break-word;
    overflow-wrap: anywhere;

    :deep(.chat-row__code-block) {
      background: rgb(var(--color-bg));
      padding: 10px 12px;
      border-radius: 6px;
      overflow-x: auto;
      margin: 8px 0;
      font-size: 12px;
      font-family: 'SF Mono', Monaco, Consolas, monospace;

      code {
        background: none;
        padding: 0;
      }
    }

    :deep(.chat-row__inline-code) {
      background: rgb(var(--color-panel-muted));
      padding: 2px 5px;
      border-radius: 4px;
      font-size: 12px;
      font-family: 'SF Mono', Monaco, Consolas, monospace;
    }

    :deep(.chat-row__cursor) {
      display: inline-block;
      width: 2px;
      height: 14px;
      background: rgb(var(--color-accent));
      margin-left: 1px;
      vertical-align: text-bottom;
      animation: blink 1s step-end infinite;
    }
  }
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

@keyframes blink {
  0%, 50% {
    opacity: 1;
  }
  51%, 100% {
    opacity: 0;
  }
}
</style>
