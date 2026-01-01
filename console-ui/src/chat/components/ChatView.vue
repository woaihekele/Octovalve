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

    <div ref="messagesContainer" class="chat-view__messages" @scroll="handleScroll">
      <div v-if="!hasMessages" class="chat-view__welcome">
        <n-icon :component="SparklesOutline" :size="48" class="chat-view__welcome-icon" />
        <h3>{{ greeting }}</h3>
        <p>开始对话来与 AI 助手交互</p>
      </div>
      <template v-else>
        <ChatMessage
          v-for="message in messages"
          :key="message.id"
          :message="message"
          :show-actions="isAskingForApproval && message.id === lastMessage?.id"
          @approve="handleApprove(message.id)"
          @reject="handleReject(message.id)"
        />
      </template>
      <div ref="scrollAnchor" class="chat-view__scroll-anchor" />
    </div>

    <ChatInput
      ref="chatInputRef"
      v-model="inputValue"
      :is-streaming="isStreaming"
      :disabled="!isConnected"
      :selected-images="selectedImages"
      :selected-files="selectedFiles"
      placeholder="输入消息，按 Enter 发送..."
      @send="handleSend"
      @cancel="handleCancel"
      @remove-image="removeImage"
      @remove-file="removeFile"
    />
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted } from 'vue';
import { NButton, NIcon } from 'naive-ui';
import {
  ChatbubblesOutline,
  AddOutline,
  TrashOutline,
  SparklesOutline,
} from '@vicons/ionicons5';
import ChatMessage from './ChatMessage.vue';
import ChatInput from './ChatInput.vue';
import { useChatService } from '../composables/useChatService';
import { useChatState } from '../composables/useChatState';

interface Props {
  title?: string;
  greeting?: string;
}

const props = withDefaults(defineProps<Props>(), {
  title: 'AI 助手',
  greeting: '你好，我是 AI 助手',
});

const {
  messages,
  isStreaming,
  isConnected,
  sendMessage,
  cancelCurrentRequest,
  approveAction,
  rejectAction,
  createSession,
  clearMessages,
} = useChatService();

const chatState = useChatState(() => messages.value ?? []);

const {
  inputValue,
  selectedImages,
  selectedFiles,
  lastMessage,
  isAskingForApproval,
  hasMessages,
  clearInput,
  removeImage,
  removeFile,
} = chatState;

const messagesContainer = ref<HTMLElement | null>(null);
const scrollAnchor = ref<HTMLElement | null>(null);
const chatInputRef = ref<InstanceType<typeof ChatInput> | null>(null);
const shouldAutoScroll = ref(true);

async function handleSend(content: string) {
  if (!content.trim()) return;

  clearInput();
  shouldAutoScroll.value = true;

  await sendMessage({
    content,
    images: selectedImages.value.length > 0 ? [...selectedImages.value] : undefined,
    files: selectedFiles.value.length > 0 ? [...selectedFiles.value] : undefined,
  });
}

function handleCancel() {
  cancelCurrentRequest();
}

function handleNewSession() {
  createSession();
  clearInput();
}

function handleClearMessages() {
  clearMessages();
  clearInput();
}

function handleApprove(messageId: string) {
  approveAction(messageId);
}

function handleReject(messageId: string) {
  rejectAction(messageId);
}

function handleScroll() {
  if (!messagesContainer.value) return;
  const { scrollTop, scrollHeight, clientHeight } = messagesContainer.value;
  shouldAutoScroll.value = scrollHeight - scrollTop - clientHeight < 100;
}

function scrollToBottom() {
  if (shouldAutoScroll.value && scrollAnchor.value) {
    scrollAnchor.value.scrollIntoView({ behavior: 'smooth' });
  }
}

watch(
  messages,
  () => {
    nextTick(scrollToBottom);
  },
  { deep: true }
);

watch(
  isStreaming,
  (streaming) => {
    if (streaming) {
      nextTick(scrollToBottom);
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
