<script setup lang="ts">
import type { TargetInfo } from '../types';

type ConnectionState = 'connected' | 'connecting' | 'disconnected';

const props = defineProps<{
  targets: TargetInfo[];
  selectedTargetName: string | null;
  pendingTotal: number;
  connectionState: ConnectionState;
}>();

const emit = defineEmits<{
  (e: 'select', name: string): void;
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
</script>

<template>
  <aside class="w-72 bg-slate-900 border-r border-slate-800 flex flex-col h-full">
    <div class="p-4 border-b border-slate-800 flex items-center gap-2">
      <div class="w-2.5 h-2.5 rounded-full bg-indigo-400"></div>
      <h1 class="font-semibold text-lg tracking-tight">Octovalve Console</h1>
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

    <div class="p-4 border-t border-slate-800 text-xs text-slate-500 text-center">
      Pending Total: {{ props.pendingTotal }}
    </div>
  </aside>
</template>
