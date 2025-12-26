<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { approveCommand, denyCommand, fetchSnapshot, fetchTargets, openConsoleSocket } from './api';
import Sidebar from './components/Sidebar.vue';
import TargetView from './components/TargetView.vue';
import SettingsModal from './components/SettingsModal.vue';
import ToastNotification from './components/ToastNotification.vue';
import { loadSettings, saveSettings } from './settings';
import type { AppSettings, ConsoleEvent, ServiceSnapshot, TargetInfo } from './types';

const targets = ref<TargetInfo[]>([]);
const snapshots = ref<Record<string, ServiceSnapshot>>({});
const selectedTargetName = ref<string | null>(null);
const settings = ref(loadSettings());
const isSettingsOpen = ref(false);
const notification = ref<{ message: string; count?: number } | null>(null);
const connectionState = ref<'connected' | 'connecting' | 'disconnected'>('connecting');

let ws: WebSocket | null = null;
let reconnectTimer: number | null = null;
const lastPendingCounts = ref<Record<string, number>>({});

const pendingTotal = computed(() => targets.value.reduce((sum, target) => sum + target.pending_count, 0));
const selectedTarget = computed(() => targets.value.find((target) => target.name === selectedTargetName.value) ?? null);
const selectedSnapshot = computed(() => {
  if (!selectedTargetName.value) {
    return null;
  }
  return snapshots.value[selectedTargetName.value] ?? null;
});

const connectionLabel = computed(() => {
  if (connectionState.value === 'connected') return '已连接';
  if (connectionState.value === 'connecting') return '连接中';
  return '连接中断';
});

const connectionBadgeClass = computed(() => {
  if (connectionState.value === 'connected') return 'bg-emerald-500/20 text-emerald-300 border-emerald-500/30';
  if (connectionState.value === 'connecting') return 'bg-amber-500/20 text-amber-300 border-amber-500/30';
  return 'bg-rose-500/20 text-rose-300 border-rose-500/30';
});

function showNotification(message: string, count?: number) {
  notification.value = { message, count };
  window.setTimeout(() => {
    notification.value = null;
  }, 4000);
}

function updateTargets(list: TargetInfo[]) {
  targets.value = list;
  if (!selectedTargetName.value && list.length > 0) {
    selectedTargetName.value = list[0].name;
  }
  list.forEach((target) => {
    if (!(target.name in lastPendingCounts.value)) {
      lastPendingCounts.value[target.name] = target.pending_count;
    }
  });
}

function applyTargetUpdate(target: TargetInfo) {
  const index = targets.value.findIndex((item) => item.name === target.name);
  if (index === -1) {
    targets.value.push(target);
  } else {
    targets.value.splice(index, 1, target);
  }

  const previous = lastPendingCounts.value[target.name] ?? 0;
  if (settings.value.notificationsEnabled && target.pending_count > previous) {
    showNotification(`收到 ${target.name} 的新请求`, target.pending_count);
  }
  lastPendingCounts.value[target.name] = target.pending_count;

  if (selectedTargetName.value === target.name) {
    const snapshot = snapshots.value[target.name];
    if (!snapshot || snapshot.queue.length !== target.pending_count) {
      refreshSnapshot(target.name);
    }
  }
}

function handleEvent(event: ConsoleEvent) {
  if (event.type === 'targets_snapshot') {
    updateTargets(event.targets);
    return;
  }
  if (event.type === 'target_updated') {
    applyTargetUpdate(event.target);
  }
}

function connectWebSocket() {
  if (ws) {
    ws.close();
  }
  connectionState.value = 'connecting';
  ws = openConsoleSocket(handleEvent);
  ws.onopen = () => {
    connectionState.value = 'connected';
  };
  ws.onclose = () => {
    connectionState.value = 'disconnected';
    scheduleReconnect();
  };
  ws.onerror = () => {
    connectionState.value = 'disconnected';
    scheduleReconnect();
  };
}

function scheduleReconnect() {
  if (reconnectTimer) {
    return;
  }
  reconnectTimer = window.setTimeout(() => {
    reconnectTimer = null;
    connectWebSocket();
  }, 3000);
}

async function refreshTargets() {
  try {
    const list = await fetchTargets();
    updateTargets(list);
  } catch (err) {
    connectionState.value = 'disconnected';
  }
}

async function refreshSnapshot(name: string) {
  try {
    const snapshot = await fetchSnapshot(name);
    snapshots.value = { ...snapshots.value, [name]: snapshot };
    lastPendingCounts.value[name] = snapshot.queue.length;
  } catch {
    // ignore fetch errors; connection status handled by websocket
  }
}

async function approve(id: string) {
  if (!selectedTargetName.value) return;
  try {
    await approveCommand(selectedTargetName.value, id);
  } catch (err) {
    showNotification('审批失败，请检查 console 服务');
  }
}

async function deny(id: string) {
  if (!selectedTargetName.value) return;
  try {
    await denyCommand(selectedTargetName.value, id);
  } catch (err) {
    showNotification('拒绝失败，请检查 console 服务');
  }
}

function handleSettingsSave(value: AppSettings) {
  settings.value = value;
  isSettingsOpen.value = false;
}

function handleGlobalKey(event: KeyboardEvent) {
  if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) {
    return;
  }
  if (event.altKey && event.key.toLowerCase() === settings.value.shortcuts.jumpNextPending.toLowerCase()) {
    event.preventDefault();
    const target = targets.value.find((item) => item.pending_count > 0);
    if (target) {
      selectedTargetName.value = target.name;
    } else {
      showNotification('没有待审批任务');
    }
  }
}

onMounted(async () => {
  await refreshTargets();
  connectWebSocket();
  window.addEventListener('keydown', handleGlobalKey);
});

onBeforeUnmount(() => {
  if (ws) {
    ws.close();
  }
  if (reconnectTimer) {
    window.clearTimeout(reconnectTimer);
  }
  window.removeEventListener('keydown', handleGlobalKey);
});

watch(selectedTargetName, (value) => {
  if (value) {
    refreshSnapshot(value);
  }
});

watch(
  settings,
  (value) => {
    saveSettings(value);
  },
  { deep: true }
);
</script>

<template>
  <div class="flex h-screen w-screen bg-slate-950 text-slate-100 overflow-hidden">
    <ToastNotification
      v-if="notification"
      :message="notification.message"
      :count="notification.count"
    />

    <Sidebar
      :targets="targets"
      :selected-target-name="selectedTargetName"
      :pending-total="pendingTotal"
      @select="selectedTargetName = $event"
    />

    <div class="flex-1 flex flex-col min-w-0 relative">
      <div class="absolute top-4 right-4 z-20 flex items-center gap-3">
        <span class="text-xs px-2 py-1 rounded border" :class="connectionBadgeClass">{{ connectionLabel }}</span>
        <button
          class="p-2 text-slate-400 hover:text-white bg-slate-900/60 hover:bg-slate-800 rounded-full transition-colors border border-slate-800"
          @click="isSettingsOpen = true"
        >
          设置
        </button>
      </div>

      <div class="flex-1">
        <TargetView
          v-if="selectedTarget"
          :target="selectedTarget"
          :snapshot="selectedSnapshot"
          :settings="settings"
          @approve="approve"
          @deny="deny"
        />
        <div v-else class="flex-1 flex items-center justify-center text-slate-600">
          请选择目标开始操作。
        </div>
      </div>
    </div>

    <SettingsModal
      :is-open="isSettingsOpen"
      :settings="settings"
      @close="isSettingsOpen = false"
      @save="handleSettingsSave"
    />
  </div>
</template>
