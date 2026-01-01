<template>
  <div
    class="chat-panel"
    :class="{ 'chat-panel--open': isOpen }"
  >
    <div class="chat-panel__content">
      <div class="chat-panel__header">
        <div class="chat-panel__header-left">
          <div class="chat-panel__header-info">
            <div class="chat-panel__title-row">
              <span class="chat-panel__title-text">{{ title }}</span>
              <span class="chat-panel__provider-badge">{{ provider === 'acp' ? 'ACP' : 'API' }}</span>
            </div>
            <div class="chat-panel__status">
              <span class="chat-panel__status-dot" :class="{ 'chat-panel__status-dot--connected': isConnected }"></span>
              <span class="chat-panel__status-text">{{ isConnected ? '已连接' : '未连接' }}</span>
            </div>
          </div>
        </div>
        <div class="chat-panel__actions">
          <button class="chat-panel__btn" title="新会话" @click="$emit('new-session')">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
          </button>
          <button class="chat-panel__btn" title="清空消息" @click="$emit('clear')">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
          </button>
          <button class="chat-panel__btn" title="关闭" @click="$emit('close')">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
          </button>
        </div>
      </div>

      <div class="chat-panel__messages" ref="messagesRef" @scroll="handleScroll">
        <div ref="contentRef" class="chat-panel__messages-content">
          <div v-if="messages.length === 0" class="chat-panel__welcome">
            <div class="chat-panel__welcome-icon">✨</div>
            <h3>{{ greeting }}</h3>
            <p>开始对话来与 AI 助手交互</p>
          </div>
          <template v-else>
            <ChatMessageRow
              v-for="message in messages"
              :key="message.id"
              :message="message"
              :is-last="message.id === messages[messages.length - 1]?.id"
              :register-bubble="registerBubble"
              :bubble-style="bubbleStyle"
            />
          </template>
          <div ref="scrollAnchor" class="chat-panel__scroll-anchor"></div>
        </div>
      </div>

      <ChatInput
        v-model="inputValue"
        :placeholder="placeholder"
        :disabled="!isConnected"
        :is-streaming="isStreaming"
        :provider="provider"
        @send="handleSend"
        @cancel="$emit('cancel')"
        @change-provider="$emit('change-provider', $event)"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, ref, watch, nextTick } from 'vue';
import ChatMessageRow from './ChatMessageRow.vue';
import ChatInput from './ChatInput.vue';
import type { ChatMessage } from '../types';
import { useStickToBottom } from '../composables/useStickToBottom';

interface Props {
  isOpen: boolean;
  title?: string;
  greeting?: string;
  placeholder?: string;
  width?: number;
  messages: ChatMessage[];
  isStreaming?: boolean;
  isConnected?: boolean;
  provider?: 'acp' | 'openai';
}

const props = withDefaults(defineProps<Props>(), {
  title: 'AI 助手',
  greeting: '你好，我是 AI 助手',
  placeholder: '输入消息，按 Enter 发送...',
  width: 380,
  isStreaming: false,
  isConnected: true,
  provider: 'acp',
});

const emit = defineEmits<{
  close: [];
  send: [content: string];
  cancel: [];
  'new-session': [];
  clear: [];
  'change-provider': [provider: 'acp' | 'openai'];
}>();

const inputValue = ref('');
const messagesRef = ref<HTMLElement | null>(null);
const contentRef = ref<HTMLElement | null>(null);
const scrollAnchor = ref<HTMLElement | null>(null);

const { scrollToBottom, handleScroll, activateStickToBottom } = useStickToBottom(
  messagesRef,
  contentRef
);

// Track per-message max width so bubbles can grow but never shrink.
const bubbleWidths = ref<Record<string, number>>({});
const bubbleElements = new Map<string, HTMLElement>();
const bubbleObservers = new Map<string, ResizeObserver>();

const updateBubbleWidth = (messageId: string, width: number) => {
  if (!Number.isFinite(width) || width <= 0) return;
  const current = bubbleWidths.value[messageId] ?? 0;
  if (width > current) {
    bubbleWidths.value = { ...bubbleWidths.value, [messageId]: width };
  }
};

const registerBubble = (messageId: string, el: HTMLElement | null) => {
  const prevEl = bubbleElements.get(messageId) || null;
  if (prevEl === el) {
    return;
  }
  if (prevEl) {
    const prevObserver = bubbleObservers.get(messageId);
    if (prevObserver) {
      prevObserver.disconnect();
      bubbleObservers.delete(messageId);
    }
    bubbleElements.delete(messageId);
  }
  if (!el) {
    return;
  }
  bubbleElements.set(messageId, el);
  if (typeof ResizeObserver !== 'undefined') {
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        updateBubbleWidth(messageId, entry.contentRect.width);
      }
    });
    observer.observe(el);
    bubbleObservers.set(messageId, observer);
  } else {
    requestAnimationFrame(() => {
      updateBubbleWidth(messageId, el.getBoundingClientRect().width);
    });
  }
};

const bubbleStyle = (messageId: string) => {
  const width = bubbleWidths.value[messageId];
  return width ? { minWidth: `${width}px` } : undefined;
};

onBeforeUnmount(() => {
  bubbleObservers.forEach((observer) => observer.disconnect());
  bubbleObservers.clear();
  bubbleElements.clear();
});

function handleSend(content: string) {
  activateStickToBottom();
  emit('send', content);
  inputValue.value = '';
}

watch(
  () => props.messages.length,
  () => {
    nextTick(() => {
      void scrollToBottom();
    });
  }
);

watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      nextTick(() => {
        activateStickToBottom();
        void scrollToBottom({ force: true, behavior: 'smooth' });
      });
    }
  }
);
</script>

<style scoped lang="scss">
.chat-panel {
  height: 100%;
  width: 0;
  background: rgb(var(--color-panel));
  border-left: 1px solid rgb(var(--color-border));
  transition: width 0.25s ease;
  overflow: hidden;
  flex-shrink: 0;

  &--open {
    width: 380px;
    min-width: 320px;
  }

  &__content {
    display: flex;
    flex-direction: column;
    height: 100%;
    width: 380px;
    opacity: 0;
    transition: opacity 0.15s ease 0.1s;

    .chat-panel--open & {
      opacity: 1;
    }
  }

  &__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px;
    background: linear-gradient(135deg, #6366f1 0%, #8b5cf6 50%, #a855f7 100%);
    flex-shrink: 0;
  }

  &__header-left {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  &__avatar {
    width: 36px;
    height: 36px;
    background: rgba(255, 255, 255, 0.2);
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    backdrop-filter: blur(4px);
    border: 1px solid rgba(255, 255, 255, 0.15);
  }

  &__avatar-icon {
    font-size: 16px;
  }

  &__header-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  &__title-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  &__title-text {
    font-weight: 600;
    font-size: 14px;
    color: white;
    letter-spacing: 0.3px;
  }

  &__provider-badge {
    font-size: 10px;
    font-weight: 500;
    padding: 2px 6px;
    background: rgba(0, 0, 0, 0.2);
    color: rgba(255, 255, 255, 0.9);
    border-radius: 4px;
    font-family: monospace;
  }

  &__status {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  &__status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.4);

    &--connected {
      background: #4ade80;
      box-shadow: 0 0 6px rgba(74, 222, 128, 0.6);
    }
  }

  &__status-text {
    font-size: 11px;
    color: rgba(255, 255, 255, 0.75);
  }

  &__actions {
    display: flex;
    gap: 4px;
  }

  &__btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    background: rgba(255, 255, 255, 0.1);
    color: rgba(255, 255, 255, 0.85);
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s;

    &:hover {
      background: rgba(255, 255, 255, 0.2);
      color: white;
    }
  }

  &__messages {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 0;

    &::-webkit-scrollbar {
      width: 8px;
    }

    &::-webkit-scrollbar-track {
      background: transparent;
    }

    &::-webkit-scrollbar-thumb {
      background: rgb(var(--color-border));
      border-radius: 4px;

      &:hover {
        background: rgb(var(--color-text-muted));
      }
    }
  }

  &__welcome {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    text-align: center;
    padding: 20px;
    color: rgb(var(--color-text-muted));

    h3 {
      margin: 12px 0 6px;
      font-size: 14px;
      font-weight: 500;
      color: rgb(var(--color-text));
    }

    p {
      margin: 0;
      font-size: 12px;
    }
  }

  &__welcome-icon {
    font-size: 32px;
    opacity: 0.6;
  }

  &__scroll-anchor {
    height: 1px;
  }

  &__messages-content {
    display: flex;
    flex-direction: column;
    min-height: 100%;
  }
}
</style>
