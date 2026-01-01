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
        <div class="chat-panel__toolbar">
          <div class="chat-panel__provider-select-wrapper">
            <svg class="chat-panel__provider-icon" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/>
            </svg>
            <select 
              class="chat-panel__provider-select"
              :value="provider"
              @change="$emit('change-provider', ($event.target as HTMLSelectElement).value as 'acp' | 'openai')"
            >
              <option value="acp">Codex CLI (ACP)</option>
              <option value="openai">OpenAI API</option>
            </select>
            <svg class="chat-panel__provider-chevron" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <polyline points="6 9 12 15 18 9"/>
            </svg>
          </div>
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

  &__input-area {
    padding: 12px 14px 14px;
    border-top: 1px solid rgb(var(--color-border));
    background: rgb(var(--color-panel));
    flex-shrink: 0;
  }

  &__input-wrapper {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    background: #f9fafb;
    border: 1px solid #e5e7eb;
    border-radius: 12px;
    padding: 10px 12px;
    transition: all 0.2s ease;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.04);

    &:focus-within {
      border-color: #8b5cf6;
      box-shadow: 0 0 0 3px rgba(139, 92, 246, 0.1);
      background: white;
    }
  }

  &__textarea {
    flex: 1;
    border: none;
    background: transparent;
    color: rgb(var(--color-text));
    font-size: 14px;
    line-height: 1.5;
    resize: none;
    outline: none;
    min-height: 22px;
    max-height: 120px;

    &::placeholder {
      color: #9ca3af;
    }

    &:disabled {
      opacity: 0.5;
    }
  }

  &__send-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border: none;
    background: linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%);
    color: white;
    border-radius: 8px;
    cursor: pointer;
    transition: all 0.2s ease;
    flex-shrink: 0;
    box-shadow: 0 2px 6px rgba(99, 102, 241, 0.3);

    &:hover:not(:disabled) {
      transform: translateY(-1px);
      box-shadow: 0 4px 12px rgba(99, 102, 241, 0.4);
    }

    &:active:not(:disabled) {
      transform: translateY(0);
    }

    &:disabled {
      opacity: 0.4;
      cursor: not-allowed;
      box-shadow: none;
    }

    &--stop {
      background: linear-gradient(135deg, #ef4444 0%, #dc2626 100%);
      box-shadow: 0 2px 6px rgba(239, 68, 68, 0.3);

      &:hover:not(:disabled) {
        box-shadow: 0 4px 12px rgba(239, 68, 68, 0.4);
      }
    }
  }

  &__disclaimer {
    text-align: center;
    font-size: 10px;
    color: #9ca3af;
    margin-top: 8px;
  }

  &__toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 8px;
  }

  &__provider-select-wrapper {
    position: relative;
    display: flex;
    align-items: center;
  }

  &__provider-icon {
    position: absolute;
    left: 8px;
    color: #6b7280;
    pointer-events: none;
  }

  &__provider-select {
    appearance: none;
    padding: 5px 28px 5px 26px;
    border: 1px solid #e5e7eb;
    background: white;
    color: #374151;
    font-size: 11px;
    font-weight: 500;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s;
    outline: none;

    &:hover {
      border-color: #d1d5db;
      background: #f9fafb;
    }

    &:focus {
      border-color: #8b5cf6;
      box-shadow: 0 0 0 2px rgba(139, 92, 246, 0.1);
    }
  }

  &__provider-chevron {
    position: absolute;
    right: 8px;
    color: #9ca3af;
    pointer-events: none;
  }
}
</style>
