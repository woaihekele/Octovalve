<template>
  <ChatPanel
    :is-open="isChatOpen"
    :min-width="chatMinWidth"
    :max-width="chatMaxWidth"
    :disable-transition="disableTransition"
    :show-drop-hint="showDropHint"
    :messages="messages"
    :plan-entries="planEntries"
    :is-streaming="isStreaming"
    :is-connected="isConnected"
    :input-locked="inputLocked"
    :provider="provider"
    :send-on-enter="sendOnEnter"
    :supports-images="supportsImages"
    :targets="targets"
    :title="$t('chat.title')"
    :greeting="$t('chat.greeting')"
    @send="emit('send', $event)"
    @cancel="emit('cancel')"
    @show-history="emit('show-history')"
    @clear="emit('clear')"
    @change-provider="emit('change-provider', $event)"
    @width-change="emit('width-change', $event)"
  />

  <ChatHistoryModal
    :show="isHistoryOpen"
    :sessions="historySessions"
    :loading="historyLoading"
    :active-session-id="activeSessionId"
    :provider="provider"
    @close="emit('close-history')"
    @select="emit('select-session', $event)"
    @delete="emit('delete-session', $event)"
    @clear-all="emit('clear-sessions')"
  />
</template>

<script setup lang="ts">
import { ChatPanel } from '../../domain/chat';
import ChatHistoryModal from '../../domain/chat/components/ChatHistoryModal.vue';
import type { ChatMessage, ChatSession, PlanEntry, SendMessageOptions } from '../../domain/chat/types';
import type { TargetInfo } from '../../shared/types';

defineProps<{
  isChatOpen: boolean;
  showDropHint: boolean;
  chatMinWidth: number;
  chatMaxWidth: number;
  disableTransition: boolean;
  messages: ChatMessage[];
  planEntries: PlanEntry[];
  isStreaming: boolean;
  isConnected: boolean;
  inputLocked: boolean;
  provider: 'acp' | 'openai';
  sendOnEnter: boolean;
  targets: TargetInfo[];
  supportsImages: boolean;
  isHistoryOpen: boolean;
  historySessions: ChatSession[];
  historyLoading: boolean;
  activeSessionId: string | null;
}>();

const emit = defineEmits<{
  (e: 'send', options: SendMessageOptions): void;
  (e: 'cancel'): void;
  (e: 'show-history'): void;
  (e: 'clear'): void;
  (e: 'change-provider', provider: 'acp' | 'openai'): void;
  (e: 'width-change', width: number): void;
  (e: 'close-history'): void;
  (e: 'select-session', sessionId: string): void;
  (e: 'delete-session', sessionId: string): void;
  (e: 'clear-sessions'): void;
}>();
</script>
