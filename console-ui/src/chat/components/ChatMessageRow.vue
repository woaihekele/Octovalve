<template>
  <div class="chat-row" :class="[`chat-row--${message.role}`, { 'chat-row--streaming': isStreaming }]">
    <div class="chat-row__avatar" v-if="message.role === 'assistant'">
      <span class="chat-row__avatar-icon">✨</span>
    </div>
    <div class="chat-row__content">
      <!-- Thinking section (collapsible) -->
      <div v-if="thinkingContent" class="chat-row__thinking" :class="{ 'chat-row__thinking--open': thinkingOpen }">
        <button class="chat-row__thinking-toggle" @click="thinkingOpen = !thinkingOpen">
          <span class="chat-row__thinking-icon">{{ thinkingOpen ? '▼' : '▶' }}</span>
          <span class="chat-row__thinking-label">{{ thinkingOpen ? '隐藏思考过程' : '显示思考过程' }}</span>
          <span v-if="isStreaming && !responseContent" class="chat-row__thinking-spinner"></span>
        </button>
        <div v-show="thinkingOpen" class="chat-row__thinking-content" v-html="renderedThinking"></div>
      </div>
      <!-- Main response bubble -->
      <div class="chat-row__bubble" v-if="responseContent || message.role === 'user' || (!thinkingContent && !responseContent)">
        <div v-if="message.role === 'assistant' && isStreaming && !responseContent && !thinkingContent" class="chat-row__typing">
          <span></span><span></span><span></span>
        </div>
        <div v-else class="chat-row__text" v-html="renderedResponse"></div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue';
import { Marked } from 'marked';
import { markedHighlight } from 'marked-highlight';
import hljs from 'highlight.js';
import type { ChatMessage } from '../types';

// Configure marked with highlight.js
const markedInstance = new Marked(
  markedHighlight({
    emptyLangClass: 'hljs',
    langPrefix: 'hljs language-',
    highlight(code, lang) {
      if (lang && hljs.getLanguage(lang)) {
        try {
          return hljs.highlight(code, { language: lang }).value;
        } catch {
          return code;
        }
      }
      return hljs.highlightAuto(code).value;
    },
  })
);
markedInstance.setOptions({ breaks: true, gfm: true });

interface Props {
  message: ChatMessage;
  isLast?: boolean;
}

const props = withDefaults(defineProps<Props>(), {
  isLast: false,
});

const thinkingOpen = ref(false);

const isStreaming = computed(() => {
  return props.message.status === 'streaming' && props.isLast;
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

const thinkingContent = computed(() => parsedContent.value.thinking);
const responseContent = computed(() => parsedContent.value.response);

const renderedThinking = computed(() => {
  if (!thinkingContent.value) return '';
  return markedInstance.parse(thinkingContent.value) as string;
});

const renderedResponse = computed(() => {
  const content = responseContent.value;
  if (!content && isStreaming.value) {
    return '<span class="chat-row__cursor"></span>';
  }
  
  let html = markedInstance.parse(content) as string;
  
  if (isStreaming.value) {
    html += '<span class="chat-row__cursor"></span>';
  }
  
  return html;
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
</style>
