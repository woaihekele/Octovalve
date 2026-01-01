<template>
  <div
    class="chat-panel"
    :class="{ 'chat-panel--open': isOpen }"
  >
    <div class="chat-panel__content">
      <div class="chat-panel__header">
        <div class="chat-panel__title">
          <span class="chat-panel__icon">✨</span>
          <span>{{ title }}</span>
        </div>
        <div class="chat-panel__actions">
          <button class="chat-panel__btn" title="新会话" @click="$emit('new-session')">＋</button>
          <button class="chat-panel__btn" title="清空消息" @click="$emit('clear')">🗑</button>
          <button class="chat-panel__btn" title="关闭" @click="$emit('close')">✕</button>
        </div>
      </div>

      <div class="chat-panel__messages" ref="messagesRef">
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
          />
        </template>
        <div ref="scrollAnchor" class="chat-panel__scroll-anchor"></div>
      </div>

      <div class="chat-panel__input-area">
        <div class="chat-panel__input-wrapper">
          <textarea
            ref="textareaRef"
            v-model="inputValue"
            class="chat-panel__textarea"
            :placeholder="placeholder"
            :disabled="!isConnected"
            rows="1"
            @keydown="handleKeyDown"
            @input="autoResize"
          ></textarea>
          <button
            v-if="isStreaming"
            class="chat-panel__send-btn chat-panel__send-btn--stop"
            title="停止"
            @click="$emit('cancel')"
          >⏹</button>
          <button
            v-else
            class="chat-panel__send-btn"
            :disabled="!canSend"
            title="发送"
            @click="handleSend"
          >↑</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted } from 'vue';
import ChatMessageRow from './ChatMessageRow.vue';
import type { ChatMessage } from '../types';

interface Props {
  isOpen: boolean;
  title?: string;
  greeting?: string;
  placeholder?: string;
  width?: number;
  messages: ChatMessage[];
  isStreaming?: boolean;
  isConnected?: boolean;
}

const props = withDefaults(defineProps<Props>(), {
  title: 'AI 助手',
  greeting: '你好，我是 AI 助手',
  placeholder: '输入消息，按 Enter 发送...',
  width: 380,
  isStreaming: false,
  isConnected: true,
});

const emit = defineEmits<{
  close: [];
  send: [content: string];
  cancel: [];
  'new-session': [];
  clear: [];
}>();

const inputValue = ref('');
const messagesRef = ref<HTMLElement | null>(null);
const scrollAnchor = ref<HTMLElement | null>(null);
const textareaRef = ref<HTMLTextAreaElement | null>(null);

const canSend = computed(() => {
  return props.isConnected && !props.isStreaming && inputValue.value.trim().length > 0;
});

function handleKeyDown(event: KeyboardEvent) {
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault();
    handleSend();
  }
}

function handleSend() {
  if (!canSend.value) return;
  emit('send', inputValue.value.trim());
  inputValue.value = '';
  nextTick(() => {
    autoResize();
  });
}

function autoResize() {
  if (!textareaRef.value) return;
  textareaRef.value.style.height = 'auto';
  const maxHeight = 120;
  textareaRef.value.style.height = `${Math.min(textareaRef.value.scrollHeight, maxHeight)}px`;
}

function scrollToBottom() {
  if (scrollAnchor.value) {
    scrollAnchor.value.scrollIntoView({ behavior: 'smooth' });
  }
}

watch(
  () => props.messages.length,
  () => {
    nextTick(scrollToBottom);
  }
);

watch(
  () => props.messages[props.messages.length - 1]?.content,
  () => {
    if (props.isStreaming) {
      nextTick(scrollToBottom);
    }
  }
);

watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      nextTick(() => {
        textareaRef.value?.focus();
        scrollToBottom();
      });
    }
  }
);

onMounted(() => {
  if (props.isOpen) {
    textareaRef.value?.focus();
  }
});
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
    padding: 10px 12px;
    border-bottom: 1px solid rgb(var(--color-border));
    background: rgb(var(--color-panel));
    flex-shrink: 0;
  }

  &__title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
    font-size: 13px;
    color: rgb(var(--color-text));
  }

  &__icon {
    font-size: 14px;
  }

  &__actions {
    display: flex;
    gap: 2px;
  }

  &__btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: none;
    background: transparent;
    color: rgb(var(--color-text-muted));
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s;

    &:hover {
      background: rgb(var(--color-panel-muted));
      color: rgb(var(--color-text));
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

  &__input-area {
    padding: 10px 12px 12px;
    border-top: 1px solid rgb(var(--color-border));
    background: rgb(var(--color-panel));
    flex-shrink: 0;
  }

  &__input-wrapper {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    background: rgb(var(--color-panel-muted));
    border: 1px solid rgb(var(--color-border));
    border-radius: 8px;
    padding: 8px 10px;
    transition: border-color 0.15s;

    &:focus-within {
      border-color: rgb(var(--color-accent));
    }
  }

  &__textarea {
    flex: 1;
    border: none;
    background: transparent;
    color: rgb(var(--color-text));
    font-size: 13px;
    line-height: 1.5;
    resize: none;
    outline: none;
    min-height: 20px;
    max-height: 120px;

    &::placeholder {
      color: rgb(var(--color-text-muted));
    }

    &:disabled {
      opacity: 0.5;
    }
  }

  &__send-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    background: rgb(var(--color-accent));
    color: white;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s;
    flex-shrink: 0;

    &:hover:not(:disabled) {
      opacity: 0.9;
    }

    &:disabled {
      opacity: 0.4;
      cursor: not-allowed;
    }

    &--stop {
      background: rgb(var(--color-danger));
    }
  }
}
</style>
