<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import {
  approveCommand,
  denyCommand,
  fetchSnapshot,
  fetchTargets,
  getProxyConfigStatus,
  logUiEvent,
  openConsoleStream,
  type ConsoleConnectionStatus,
  type ConsoleStreamHandle,
} from './api';
import { matchesShortcut } from './shortcuts';
import Sidebar from './components/Sidebar.vue';
import TerminalPanel from './components/TerminalPanel.vue';
import TargetView from './components/TargetView.vue';
import SettingsModal from './components/SettingsModal.vue';
import ToastNotification from './components/ToastNotification.vue';
import { loadSettings, saveSettings } from './settings';
import type { AppSettings, ConsoleEvent, ServiceSnapshot, TargetInfo } from './types';
import { startWindowDrag } from './tauriWindow';

const targets = ref<TargetInfo[]>([]);
const snapshots = ref<Record<string, ServiceSnapshot>>({});
const selectedTargetName = ref<string | null>(null);
const settings = ref(loadSettings());
const isSettingsOpen = ref(false);
const notification = ref<{ message: string; count?: number } | null>(null);
const connectionState = ref<'connected' | 'connecting' | 'disconnected'>('connecting');
const snapshotLoading = ref<Record<string, boolean>>({});
const pendingJumpToken = ref(0);
const terminalState = ref<Record<string, { initialized: boolean; open: boolean }>>({});

let streamHandle: ConsoleStreamHandle | null = null;
const lastPendingCounts = ref<Record<string, number>>({});

const pendingTotal = computed(() => targets.value.reduce((sum, target) => sum + target.pending_count, 0));
const selectedTarget = computed(() => targets.value.find((target) => target.name === selectedTargetName.value) ?? null);
const selectedTerminal = computed(() => {
  if (!selectedTargetName.value) {
    return { initialized: false, open: false };
  }
  return terminalState.value[selectedTargetName.value] ?? { initialized: false, open: false };
});
const selectedSnapshot = computed(() => {
  if (!selectedTargetName.value) {
    return null;
  }
  return snapshots.value[selectedTargetName.value] ?? null;
});

const terminalEntries = computed(() =>
  targets.value
    .map((target) => ({ target, state: terminalState.value[target.name] }))
    .filter((entry) => entry.state?.initialized)
    .map((entry) => ({ target: entry.target, state: entry.state! }))
);

function openTerminalForTarget(name: string) {
  terminalState.value = {
    ...terminalState.value,
    [name]: { initialized: true, open: true },
  };
}

function closeTerminalForTarget(name: string) {
  const current = terminalState.value[name];
  if (!current) {
    return;
  }
  terminalState.value = {
    ...terminalState.value,
    [name]: { ...current, open: false },
  };
}

function handleOpenTerminal() {
  if (!selectedTargetName.value) {
    return;
  }
  openTerminalForTarget(selectedTargetName.value);
}

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
    refreshSnapshot(target.name);
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

async function connectWebSocket() {
  if (streamHandle) {
    streamHandle.close();
  }
  connectionState.value = 'connecting';
  void logUiEvent('ws connecting');
  try {
    streamHandle = await openConsoleStream(handleEvent, (status: ConsoleConnectionStatus) => {
      connectionState.value = status;
      if (status === 'connected') {
        void logUiEvent('ws connected');
      } else if (status === 'disconnected') {
        void logUiEvent('ws closed');
      }
    });
  } catch (err) {
    connectionState.value = 'disconnected';
    void logUiEvent(`ws start failed: ${String(err)}`);
  }
}

async function refreshTargets() {
  try {
    const list = await fetchTargets();
    updateTargets(list);
  } catch (err) {
    connectionState.value = 'disconnected';
    void logUiEvent(`fetch targets failed: ${String(err)}`);
  }
}

async function refreshSnapshot(name: string) {
  if (snapshotLoading.value[name]) {
    return;
  }
  snapshotLoading.value[name] = true;
  try {
    const snapshot = await fetchSnapshot(name);
    snapshots.value = { ...snapshots.value, [name]: snapshot };
    lastPendingCounts.value[name] = snapshot.queue.length;
  } catch {
    void logUiEvent(`fetch snapshot failed target=${name}`);
    // ignore fetch errors; connection status handled by websocket
  } finally {
    snapshotLoading.value[name] = false;
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
  if (isSettingsOpen.value) {
    return;
  }
  if (matchesShortcut(event, settings.value.shortcuts.prevTarget)) {
    event.preventDefault();
    if (targets.value.length === 0) {
      return;
    }
    const currentIndex = selectedTargetName.value
      ? targets.value.findIndex((item) => item.name === selectedTargetName.value)
      : -1;
    const nextIndex = Math.max(currentIndex - 1, 0);
    if (currentIndex === -1) {
      selectedTargetName.value = targets.value[0].name;
    } else if (nextIndex !== currentIndex) {
      selectedTargetName.value = targets.value[nextIndex].name;
    }
    return;
  }
  if (matchesShortcut(event, settings.value.shortcuts.nextTarget)) {
    event.preventDefault();
    if (targets.value.length === 0) {
      return;
    }
    const currentIndex = selectedTargetName.value
      ? targets.value.findIndex((item) => item.name === selectedTargetName.value)
      : -1;
    const nextIndex = Math.min(currentIndex + 1, targets.value.length - 1);
    if (currentIndex === -1) {
      selectedTargetName.value = targets.value[0].name;
    } else if (nextIndex !== currentIndex) {
      selectedTargetName.value = targets.value[nextIndex].name;
    }
    return;
  }
  if (matchesShortcut(event, settings.value.shortcuts.jumpNextPending)) {
    event.preventDefault();
    const target = targets.value.find((item) => item.pending_count > 0);
    if (target) {
      selectedTargetName.value = target.name;
      pendingJumpToken.value += 1;
    } else {
      showNotification('没有待审批任务');
    }
  }
}

function handleTitleDrag(event: MouseEvent) {
  if (event.button !== 0) {
    return;
  }
  event.preventDefault();
  void startWindowDrag();
}

onMounted(async () => {
  try {
    const status = await getProxyConfigStatus();
    if (!status.present) {
      connectionState.value = 'disconnected';
      showNotification(`未找到 ${status.path}，请参考 ${status.example_path} 创建并修改后重启应用`);
      void logUiEvent(`proxy config missing: ${status.path}`);
      return;
    }
  } catch (err) {
    void logUiEvent(`proxy config check failed: ${String(err)}`);
  }
  await refreshTargets();
  void connectWebSocket();
  void logUiEvent(`origin=${window.location.origin} secure=${window.isSecureContext}`);
  window.addEventListener('keydown', handleGlobalKey);
});

onBeforeUnmount(() => {
  if (streamHandle) {
    streamHandle.close();
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
  <div class="flex h-screen w-screen bg-slate-950 text-slate-100 overflow-hidden pt-7">
    <div
      class="fixed top-0 left-0 right-0 h-7 z-30"
      data-tauri-drag-region
      @mousedown="handleTitleDrag"
    ></div>
    <ToastNotification
      v-if="notification"
      :message="notification.message"
      :count="notification.count"
    />

    <Sidebar
      :targets="targets"
      :selected-target-name="selectedTargetName"
      :pending-total="pendingTotal"
      :connection-state="connectionState"
      @select="selectedTargetName = $event"
      @open-settings="isSettingsOpen = true"
    />

    <div class="flex-1 flex flex-col min-w-0 min-h-0 relative">
      <div class="absolute top-4 right-4 z-20 flex items-center gap-3">
        <span
          v-if="connectionState === 'disconnected'"
          class="text-xs px-2 py-1 rounded border bg-rose-500/20 text-rose-300 border-rose-500/30"
        >
          console 异常，请重启
        </span>
      </div>

      <div class="flex-1 min-h-0">
        <TargetView
          v-if="selectedTarget"
          :target="selectedTarget"
          :snapshot="selectedSnapshot"
          :settings="settings"
          :pending-jump-token="pendingJumpToken"
          :terminal-open="selectedTerminal.open"
          @approve="approve"
          @deny="deny"
          @open-terminal="handleOpenTerminal"
        />
        <div v-else class="flex-1 flex items-center justify-center text-slate-600">
          请选择目标开始操作。
        </div>
      </div>
      <TerminalPanel
        v-for="entry in terminalEntries"
        :key="entry.target.name"
        :target="entry.target"
        :visible="entry.state.open && selectedTargetName === entry.target.name"
        @close="closeTerminalForTarget(entry.target.name)"
      />
    </div>

    <SettingsModal
      :is-open="isSettingsOpen"
      :settings="settings"
      @close="isSettingsOpen = false"
      @save="handleSettingsSave"
    />
  </div>
</template>
