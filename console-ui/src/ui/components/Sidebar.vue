<script setup lang="ts">
import type { TargetInfo } from '../../shared/types';

type ConnectionState = 'connected' | 'connecting' | 'disconnected';

const props = defineProps<{
  targets: TargetInfo[];
  selectedTargetName: string | null;
  pendingTotal: number;
  connectionState: ConnectionState;
}>();

const emit = defineEmits<{
  (e: 'select', name: string): void;
  (e: 'open-settings'): void;
}>();

function resolveStatus(target: TargetInfo) {
  if (target.status === 'ready') {
    return 'ready';
  }
  if (props.connectionState === 'connecting') {
    return 'connecting';
  }
  if (!target.last_seen && !target.last_error) {
    return 'connecting';
  }
  return 'down';
}

function statusLabel(target: TargetInfo) {
  const status = resolveStatus(target);
  if (status === 'ready') return 'ready';
  if (status === 'connecting') return '连接中';
  return 'down';
}

function statusClass(target: TargetInfo) {
  const status = resolveStatus(target);
  if (status === 'ready') return 'bg-success';
  if (status === 'connecting') return 'bg-warning';
  return 'bg-danger';
}
</script>

<template>
  <aside class="w-72 bg-panel border-r border-border flex flex-col h-full">
    <div
      class="p-4 border-b border-border flex items-center gap-2"
      data-tauri-drag-region
    >
      <div class="w-2.5 h-2.5 rounded-full bg-accent"></div>
      <h1 class="font-semibold text-lg tracking-tight">Octovalve</h1>
    </div>

    <div class="flex-1 overflow-y-auto p-2 space-y-1">
      <button
        v-for="target in props.targets"
        :key="target.name"
        type="button"
        @click="emit('select', target.name)"
        class="w-full text-left p-3 rounded-lg transition-colors flex items-center justify-between"
        :class="props.selectedTargetName === target.name ? 'bg-panel-muted border border-border' : 'hover:bg-panel-muted/50 border border-transparent'"
      >
        <div class="flex flex-col min-w-0">
          <div class="flex items-center gap-2">
            <span class="font-medium text-sm truncate text-foreground">{{ target.name }}</span>
            <span v-if="target.is_default" class="text-[10px] px-1.5 py-0.5 rounded bg-accent/20 text-accent">默认</span>
          </div>
          <div class="flex items-center gap-2 mt-1 text-xs text-foreground-muted">
            <span class="truncate">
              {{ target.hostname || target.ip || target.desc }}
            </span>
          </div>
          <div class="flex items-center gap-2 mt-1 text-xs text-foreground-muted">
            <span class="inline-flex items-center gap-1">
              <span class="h-2 w-2 rounded-full" :class="statusClass(target)"></span>
              <span class="capitalize">{{ statusLabel(target) }}</span>
            </span>
          </div>
        </div>

        <div
          v-if="target.pending_count > 0"
          class="bg-danger text-white text-xs font-semibold px-2 py-0.5 rounded-full min-w-[20px] text-center shadow-sm shadow-danger/40"
        >
          {{ target.pending_count }}
        </div>
      </button>
    </div>

    <div class="p-4 border-t border-border flex items-center justify-between text-xs text-foreground-muted">
      <button
        class="p-2 rounded-full border border-border text-foreground-muted hover:text-foreground hover:border-accent/40 transition-colors"
        @click="emit('open-settings')"
        aria-label="设置"
        title="设置"
      >
        <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3"></circle>
          <path
            d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33 1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82 1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"
          ></path>
        </svg>
      </button>
    </div>
  </aside>
</template>
