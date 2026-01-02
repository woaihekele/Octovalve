<template>
  <div class="chat-view">
    <div class="chat-view__header">
      <div class="chat-view__title">
        <n-icon :component="ChatbubblesOutline" :size="20" />
        <span>{{ title }}</span>
      </div>
      <div class="chat-view__actions">
        <n-button size="small" quaternary circle @click="handleNewSession">
          <template #icon>
            <n-icon :component="AddOutline" />
          </template>
        </n-button>
        <n-button size="small" quaternary circle @click="handleClearMessages">
          <template #icon>
            <n-icon :component="TrashOutline" />
          </template>
        </n-button>
      </div>
    </div>

    <div ref="messagesRef" class="chat-view__messages" @scroll="handleScroll">
      <div ref="contentRef" class="chat-view__messages-content">
      <div v-if="messages.length === 0" class="chat-view__welcome">
        <n-icon :component="SparklesOutline" :size="48" class="chat-view__welcome-icon" />
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
      <div ref="scrollAnchor" class="chat-view__scroll-anchor" />
      </div>
    </div>

    <ChatInput
      ref="chatInputRef"
      v-model="inputValue"
      :is-streaming="isStreaming"
      :disabled="!isConnected"
      :provider="provider"
      placeholder="输入消息，按 Enter 发送..."
      @send="handleSend"
      @cancel="handleCancel"
      @change-provider="handleChangeProvider"
    />
  </div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, ref, watch, nextTick, onMounted } from 'vue';
import { NButton, NIcon } from 'naive-ui';
import {
  ChatbubblesOutline,
  AddOutline,
  TrashOutline,
  SparklesOutline,
} from '@vicons/ionicons5';
import type { ChatMessage } from '../types';
import ChatInput from './ChatInput.vue';
import ChatMessageRow from './ChatMessageRow.vue';
import { useStickToBottom } from '../composables/useStickToBottom';

interface Props {
  isOpen?: boolean;
  title?: string;
  greeting?: string;
  messages: ChatMessage[];
  isStreaming?: boolean;
  isConnected?: boolean;
  provider?: 'acp' | 'openai';
}

const props = withDefaults(defineProps<Props>(), {
  isOpen: true,
  title: 'AI 助手',
  greeting: '你好，我是 AI 助手',
  messages: () => [],
  isStreaming: false,
  isConnected: true,
  provider: 'acp',
});

const emit = defineEmits<{
  send: [content: string];
  cancel: [];
  'new-session': [];
  clear: [];
  'change-provider': [provider: 'acp' | 'openai'];
}>();

const inputValue = ref('');

const chatInputRef = ref<InstanceType<typeof ChatInput> | null>(null);

const messagesRef = ref<HTMLElement | null>(null);
const contentRef = ref<HTMLElement | null>(null);
const scrollAnchor = ref<HTMLElement | null>(null);

const { stickToBottom, scrollToBottom, handleScroll, activateStickToBottom } = useStickToBottom(
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

async function handleSend(content: string) {
  if (!content.trim()) return;

  activateStickToBottom();
  emit('send', content);
  inputValue.value = '';
}

function handleCancel() {
  emit('cancel');
}

function handleNewSession() {
  emit('new-session');
  inputValue.value = '';
}

function handleClearMessages() {
  emit('clear');
  inputValue.value = '';
}

function handleChangeProvider(newProvider: 'acp' | 'openai') {
  emit('change-provider', newProvider);
}

watch(
  () => props.messages.length,
  () => {
    nextTick(scrollToBottom);
  },
  { flush: 'post' }
);

watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      nextTick(() => {
        activateStickToBottom();
        void scrollToBottom({ force: true, behavior: 'smooth' });
        chatInputRef.value?.focus();
      });
    }
  }
);

onMounted(() => {
  chatInputRef.value?.focus();
});
</script>

<style scoped lang="scss">
.chat-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--color-bg);
  border-radius: 8px;
  border: 1px solid var(--color-border);

  &__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-panel);
    border-radius: 8px 8px 0 0;
  }

  &__title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
    color: var(--color-text);
  }

  &__actions {
    display: flex;
    gap: 4px;
  }

  &__messages {
    flex: 1;
    overflow-y: auto;
    padding: 16px;
    min-height: 0;
  }

  &__messages-content {
    display: flex;
    flex-direction: column;
    min-height: 100%;
  }

  &__welcome {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    text-align: center;
    color: var(--color-text-muted);

    h3 {
      margin: 16px 0 8px;
      color: var(--color-text);
    }

    p {
      margin: 0;
    }
  }

  &__welcome-icon {
    color: var(--color-accent);
    opacity: 0.6;
  }

  &__scroll-anchor {
    height: 1px;
  }
}
</style>
