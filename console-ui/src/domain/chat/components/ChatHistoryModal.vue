<template>
  <n-modal :show="props.show" :mask-closable="true" :close-on-esc="true" @update:show="(v) => !v && emit('close')">
    <div class="chat-history-modal-root">
      <n-card size="small" class="w-[36rem]" :bordered="true">
        <template #header>
          <div>{{ $t('chat.history.title') }}</div>
        </template>
        <template #header-extra>
          <n-button text @click="emit('close')" :aria-label="$t('common.close')" :title="$t('common.close')">
            <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </n-button>
        </template>

      <div class="space-y-3">
        <div class="flex items-center justify-between">
          <div class="text-xs text-foreground-muted">{{ historyHint }}</div>
          <n-button size="small" quaternary :disabled="props.sessions.length === 0" @click="confirmClearAllOpen = true">
            {{ $t('chat.history.clearAll') }}
          </n-button>
        </div>

        <div v-if="props.sessions.length === 0" class="text-sm text-foreground-muted py-6 text-center">
          {{ $t('chat.history.empty') }}
        </div>

        <div v-else class="max-h-[50vh] overflow-auto scrollbar-chat rounded border border-border">
          <div
            v-for="s in sortedSessions"
            :key="s.id"
            class="flex items-center justify-between gap-3 px-3 py-2 border-b border-border last:border-b-0"
          >
            <button
              class="flex-1 text-left"
              :class="s.id === props.activeSessionId ? 'text-foreground' : 'text-foreground-muted'"
              @click="emit('select', s.id)"
            >
              <div class="text-sm font-medium truncate">{{ sessionTitle(s) }}</div>
              <div class="text-xs opacity-80 truncate">{{ formatMeta(s) }}</div>
            </button>

            <div class="flex items-center gap-2">
              <span v-if="s.id === props.activeSessionId" class="text-xs text-accent">{{ $t('chat.history.current') }}</span>
              <n-button size="small" quaternary @click="emit('delete', s.id)">{{ $t('common.delete') }}</n-button>
            </div>
          </div>
        </div>
      </div>
      </n-card>
    </div>
  </n-modal>

  <n-modal v-model:show="confirmClearAllOpen" :mask-closable="true" :close-on-esc="true">
    <n-card size="small" class="w-[22rem]" :bordered="true">
      <template #header>{{ $t('chat.history.confirmTitle') }}</template>
      <div class="text-sm text-foreground-muted">{{ $t('chat.history.confirmHint') }}</div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button @click="confirmClearAllOpen = false">{{ $t('common.cancel') }}</n-button>
          <n-button type="error" @click="confirmClearAll">{{ $t('chat.history.confirmAction') }}</n-button>
        </div>
      </template>
    </n-card>
  </n-modal>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue';
import { NButton, NCard, NModal } from 'naive-ui';
import { useI18n } from 'vue-i18n';
import type { ChatSession } from '../types';

const props = defineProps<{
  show: boolean;
  sessions: ChatSession[];
  activeSessionId: string | null;
  provider?: 'acp' | 'openai';
}>();

const emit = defineEmits<{
  (e: 'close'): void;
  (e: 'select', sessionId: string): void;
  (e: 'delete', sessionId: string): void;
  (e: 'clear-all'): void;
}>();

const sortedSessions = computed(() => {
  return [...props.sessions].sort((a, b) => b.updatedAt - a.updatedAt);
});

const { t, locale } = useI18n();
const confirmClearAllOpen = ref(false);
const historyHint = computed(() =>
  props.provider === 'acp' ? t('chat.history.hintAcp') : t('chat.history.hintOpenai')
);

function confirmClearAll() {
  confirmClearAllOpen.value = false;
  emit('clear-all');
}

function formatMeta(session: ChatSession) {
  const count = session.messageCount ?? session.messages.length;
  const dt = new Date(session.updatedAt);
  const time = dt.toLocaleString(locale.value);
  return t('chat.history.meta', { count, time });
}

function sessionTitle(session: ChatSession) {
  const firstUser = session.messages.find((m) => m.role === 'user' && m.content.trim().length > 0);
  const raw = (firstUser?.content ?? session.title).trim();
  const singleLine = raw.replace(/\s+/g, ' ');
  const maxLen = 80;
  if (singleLine.length <= maxLen) {
    return singleLine;
  }
  return `${singleLine.slice(0, maxLen - 1)}…`;
}
</script>
