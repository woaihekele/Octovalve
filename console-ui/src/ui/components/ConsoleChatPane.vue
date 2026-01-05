<template>
  <ChatPanel
    :is-open="isChatOpen"
    :messages="messages"
    :is-streaming="isStreaming"
    :is-connected="isConnected"
    :input-locked="inputLocked"
    :provider="provider"
    :send-on-enter="sendOnEnter"
    :title="$t('chat.title')"
    :greeting="$t('chat.greeting')"
    @send="emit('send', $event)"
    @cancel="emit('cancel')"
    @show-history="emit('show-history')"
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
import { ChatPanel } from '../../domain/chat';
import ChatHistoryModal from '../../domain/chat/components/ChatHistoryModal.vue';
import type { ChatMessage, ChatSession } from '../../domain/chat/types';

defineProps<{
  isChatOpen: boolean;
  messages: ChatMessage[];
  isStreaming: boolean;
  isConnected: boolean;
  inputLocked: boolean;
  provider: 'acp' | 'openai';
  sendOnEnter: boolean;
  isHistoryOpen: boolean;
  openaiSessions: ChatSession[];
  activeSessionId: string | null;
}>();

const emit = defineEmits<{
  (e: 'send', content: string): void;
  (e: 'cancel'): void;
  (e: 'show-history'): void;
  (e: 'clear'): void;
  (e: 'change-provider', provider: 'acp' | 'openai'): void;
  (e: 'close-history'): void;
  (e: 'select-session', sessionId: string): void;
  (e: 'delete-session', sessionId: string): void;
  (e: 'clear-sessions'): void;
}>();
</script>
