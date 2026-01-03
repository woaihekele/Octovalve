<template>
  <ChatPanel
    :is-open="isChatOpen"
    :messages="messages"
    :is-streaming="isStreaming"
    :is-connected="isConnected"
    :provider="provider"
    title="AI 助手"
    greeting="你好，我是 AI 助手"
    @send="emit('send', $event)"
    @cancel="emit('cancel')"
    @show-history="emit('show-history')"
    @new-session="emit('new-session')"
    @clear="emit('clear')"
    @change-provider="emit('change-provider', $event)"
  />

  <ChatHistoryModal
    :show="isHistoryOpen"
    :sessions="openaiSessions"
    :active-session-id="activeSessionId"
    @close="emit('close-history')"
    @select="emit('select-session', $event)"
    @delete="emit('delete-session', $event)"
    @clear-all="emit('clear-sessions')"
  />
</template>

<script setup lang="ts">
import { ChatPanel } from '../chat';
import ChatHistoryModal from '../chat/components/ChatHistoryModal.vue';
import type { ChatMessage, ChatSession } from '../chat/types';

defineProps<{
  isChatOpen: boolean;
  messages: ChatMessage[];
  isStreaming: boolean;
  isConnected: boolean;
  provider: 'acp' | 'openai';
  isHistoryOpen: boolean;
  openaiSessions: ChatSession[];
  activeSessionId: string | null;
}>();

const emit = defineEmits<{
  (e: 'send', content: string): void;
  (e: 'cancel'): void;
  (e: 'show-history'): void;
  (e: 'new-session'): void;
  (e: 'clear'): void;
  (e: 'change-provider', provider: 'acp' | 'openai'): void;
  (e: 'close-history'): void;
  (e: 'select-session', sessionId: string): void;
  (e: 'delete-session', sessionId: string): void;
  (e: 'clear-sessions'): void;
}>();
</script>
