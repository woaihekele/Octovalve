<script setup lang="ts">
import type { TargetInfo } from '../types';
import { startWindowDrag } from '../tauriWindow';

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
  if (status === 'ready') return 'bg-emerald-400';
  if (status === 'connecting') return 'bg-amber-400';
  return 'bg-rose-400';
}

function handleTitleDrag(event: MouseEvent) {
  if (event.button !== 0) {
    return;
  }
  event.preventDefault();
  void startWindowDrag();
}
</script>

<template>
  <aside class="w-72 bg-slate-900 border-r border-slate-800 flex flex-col h-full">
    <div
      class="p-4 border-b border-slate-800 flex items-center gap-2"
      data-tauri-drag-region
      @mousedown="handleTitleDrag"
    >
      <div class="w-2.5 h-2.5 rounded-full bg-indigo-400"></div>
      <h1 class="font-semibold text-lg tracking-tight">Octovalve</h1>
    </div>

    <div class="flex-1 overflow-y-auto p-2 space-y-1">
      <button
        v-for="target in props.targets"
        :key="target.name"
        type="button"
        @click="emit('select', target.name)"
        class="w-full text-left p-3 rounded-lg transition-colors flex items-center justify-between"
        :class="props.selectedTargetName === target.name ? 'bg-slate-800 border border-slate-700' : 'hover:bg-slate-800/50 border border-transparent'"
      >
        <div class="flex flex-col min-w-0">
          <div class="flex items-center gap-2">
            <span class="font-medium text-sm truncate text-slate-100">{{ target.name }}</span>
            <span v-if="target.is_default" class="text-[10px] px-1.5 py-0.5 rounded bg-indigo-500/20 text-indigo-300">默认</span>
          </div>
          <div class="flex items-center gap-2 mt-1 text-xs text-slate-500">
            <span class="truncate">
              {{ target.hostname || target.ip || target.desc }}
            </span>
          </div>
          <div class="flex items-center gap-2 mt-1 text-xs text-slate-500">
            <span class="inline-flex items-center gap-1">
              <span class="h-2 w-2 rounded-full" :class="statusClass(target)"></span>
              <span class="capitalize">{{ statusLabel(target) }}</span>
            </span>
          </div>
        </div>

        <div
          v-if="target.pending_count > 0"
          class="bg-rose-500 text-white text-xs font-semibold px-2 py-0.5 rounded-full min-w-[20px] text-center shadow-sm shadow-rose-900/40"
        >
          {{ target.pending_count }}
        </div>
      </button>
    </div>

    <div class="p-4 border-t border-slate-800 flex items-center justify-between text-xs text-slate-500">
      <button
        class="p-2 rounded-full border border-slate-800 text-slate-400 hover:text-white hover:border-indigo-500/40 transition-colors"
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
      <div>Pending Total: {{ props.pendingTotal }}</div>
    </div>
  </aside>
</template>
