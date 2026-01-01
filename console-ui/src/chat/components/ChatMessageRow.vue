<template>
  <div class="chat-row" :class="[`chat-row--${message.role}`, { 'chat-row--streaming': isStreaming }]">
    <div class="chat-row__avatar" v-if="message.role === 'assistant'">
      <span class="chat-row__avatar-icon">✨</span>
    </div>
    <div class="chat-row__content">
      <div class="chat-row__bubble">
        <div v-if="message.role === 'assistant' && isStreaming && !message.content" class="chat-row__typing">
          <span></span><span></span><span></span>
        </div>
        <div v-else class="chat-row__text" v-html="renderedContent"></div>
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
  display: flex;
  gap: 10px;
  padding: 12px 16px;
  align-items: flex-start;

  &--user {
    flex-direction: row-reverse;
    
    .chat-row__bubble {
      background: linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%);
      color: #fff;
      border-radius: 18px 18px 4px 18px;
      max-width: 80%;
    }
    
    .chat-row__text {
      color: #fff;
    }
  }

  &--assistant {
    .chat-row__bubble {
      background: rgb(var(--color-panel-muted));
      border-radius: 18px 18px 18px 4px;
      max-width: 90%;
    }
  }

  &__avatar {
    flex-shrink: 0;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  &__avatar-icon {
    font-size: 16px;
  }

  &__content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  &__bubble {
    padding: 10px 14px;
    transition: all 0.2s ease;
  }

  &__typing {
    display: flex;
    gap: 4px;
    padding: 4px 0;
    
    span {
      width: 6px;
      height: 6px;
      background: rgb(var(--color-text-muted));
      border-radius: 50%;
      animation: typing 1.4s ease-in-out infinite;
      
      &:nth-child(2) { animation-delay: 0.2s; }
      &:nth-child(3) { animation-delay: 0.4s; }
    }
  }

  &__text {
    font-size: 14px;
    line-height: 1.6;
    color: rgb(var(--color-text));
    word-break: break-word;
    overflow-wrap: anywhere;

    :deep(.chat-row__code-block) {
      background: rgba(0, 0, 0, 0.15);
      padding: 12px 14px;
      border-radius: 8px;
      overflow-x: auto;
      margin: 10px 0;
      font-size: 13px;
      font-family: 'SF Mono', Monaco, Consolas, monospace;

      code {
        background: none;
        padding: 0;
      }
    }

    :deep(.chat-row__inline-code) {
      background: rgba(0, 0, 0, 0.1);
      padding: 2px 6px;
      border-radius: 4px;
      font-size: 13px;
      font-family: 'SF Mono', Monaco, Consolas, monospace;
    }

    :deep(.chat-row__cursor) {
      display: inline-block;
      width: 2px;
      height: 16px;
      background: rgb(var(--color-accent));
      margin-left: 2px;
      vertical-align: text-bottom;
      animation: blink 1s step-end infinite;
    }
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

@keyframes blink {
  0%, 50% { opacity: 1; }
  51%, 100% { opacity: 0; }
}
</style>
