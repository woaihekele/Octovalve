<template>
  <n-modal :show="props.show" :mask-closable="true" :close-on-esc="true" @update:show="(v) => !v && emit('close')">
    <n-card size="small" class="w-[36rem]" :bordered="true">
      <template #header>历史会话</template>
      <template #header-extra>
        <n-button text @click="emit('close')" aria-label="关闭" title="关闭">
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </n-button>
      </template>

      <div class="space-y-3">
        <div class="flex items-center justify-between">
          <div class="text-xs text-foreground-muted">选择一个会话继续对话（API）</div>
          <n-button size="small" quaternary :disabled="props.sessions.length === 0" @click="confirmClearAllOpen = true">
            全部清空
          </n-button>
        </div>

        <div v-if="props.sessions.length === 0" class="text-sm text-foreground-muted py-6 text-center">
          暂无历史会话
        </div>

        <div v-else class="max-h-[50vh] overflow-auto rounded border border-border">
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
              <div class="text-sm font-medium truncate">{{ s.title }}</div>
              <div class="text-xs opacity-80 truncate">{{ formatMeta(s) }}</div>
            </button>

            <div class="flex items-center gap-2">
              <span v-if="s.id === props.activeSessionId" class="text-xs text-accent">当前</span>
              <n-button size="small" quaternary @click="emit('delete', s.id)">删除</n-button>
            </div>
          </div>
        </div>
      </div>
    </n-card>
  </n-modal>

  <n-modal v-model:show="confirmClearAllOpen" :mask-closable="true" :close-on-esc="true">
    <n-card size="small" class="w-[22rem]" :bordered="true">
      <template #header>确认清空</template>
      <div class="text-sm text-foreground-muted">将删除所有历史会话，且无法恢复。是否继续？</div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button @click="confirmClearAllOpen = false">取消</n-button>
          <n-button type="error" @click="confirmClearAll">确认清空</n-button>
        </div>
      </template>
    </n-card>
  </n-modal>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue';
import { NButton, NCard, NModal } from 'naive-ui';
import type { ChatSession } from '../types';

const props = defineProps<{
  show: boolean;
  sessions: ChatSession[];
  activeSessionId: string | null;
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

const confirmClearAllOpen = ref(false);

function confirmClearAll() {
  confirmClearAllOpen.value = false;
  emit('clear-all');
}

function formatMeta(session: ChatSession) {
  const count = session.messages.length;
  const dt = new Date(session.updatedAt);
  const time = dt.toLocaleString();
  return `${count} 条消息 · ${time}`;
}
</script>
