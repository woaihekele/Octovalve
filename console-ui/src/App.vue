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
import { NButton, NConfigProvider, NIcon, NNotificationProvider, NTabPane, NTabs, darkTheme } from 'naive-ui';
import { ChatbubblesOutline } from '@vicons/ionicons5';
import { ChatDrawer } from './chat';
import { matchesShortcut } from './shortcuts';
import Sidebar from './components/Sidebar.vue';
import TerminalPanel from './components/TerminalPanel.vue';
import TargetView from './components/TargetView.vue';
import SettingsModal from './components/SettingsModal.vue';
import NotificationBridge from './components/NotificationBridge.vue';
import { loadSettings, saveSettings } from './settings';
import type { AppSettings, ConsoleEvent, ServiceSnapshot, TargetInfo } from './types';
import { startWindowDrag } from './tauriWindow';
import { useAiRiskQueue } from './composables/useAiRiskQueue';
import { useTerminalState } from './composables/useTerminalState';
import { useThemeMode } from './composables/useThemeMode';

const targets = ref<TargetInfo[]>([]);
const snapshots = ref<Record<string, ServiceSnapshot>>({});
const selectedTargetName = ref<string | null>(null);
const settings = ref(loadSettings());
const isSettingsOpen = ref(false);
const isChatOpen = ref(false);
const notification = ref<{ message: string; count?: number } | null>(null);
const notificationToken = ref(0);
const connectionState = ref<'connected' | 'connecting' | 'disconnected'>('connecting');
const snapshotLoading = ref<Record<string, boolean>>({});
const pendingJumpToken = ref(0);
const { resolvedTheme, applyThemeMode } = useThemeMode();
const naiveTheme = computed(() => (resolvedTheme.value === 'light' ? null : darkTheme));
function resolveRgbVar(name: string, fallback: string) {
  if (typeof window === 'undefined' || typeof document === 'undefined') {
    return `rgb(${fallback})`;
  }
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const value = raw || fallback;
  if (value.startsWith('rgb')) {
    return value;
  }
  const normalized = value.replace(/\s+/g, ', ');
  return `rgb(${normalized})`;
}

const naiveThemeOverrides = computed(() => {
  void resolvedTheme.value;
  return {
    common: {
      primaryColor: resolveRgbVar('--color-accent', '99 102 241'),
      primaryColorHover: resolveRgbVar('--color-accent', '99 102 241'),
      primaryColorPressed: resolveRgbVar('--color-accent-soft', '67 56 202'),
      primaryColorSuppl: resolveRgbVar('--color-accent-soft', '67 56 202'),
      successColor: resolveRgbVar('--color-success', '52 211 153'),
      warningColor: resolveRgbVar('--color-warning', '251 191 36'),
      errorColor: resolveRgbVar('--color-danger', '244 63 94'),
      textColorBase: resolveRgbVar('--color-text', '226 232 240'),
      textColor1: resolveRgbVar('--color-text', '226 232 240'),
      textColor2: resolveRgbVar('--color-text-muted', '100 116 139'),
      textColor3: resolveRgbVar('--color-text-muted', '100 116 139'),
      placeholderColor: resolveRgbVar('--color-text-muted', '100 116 139'),
      borderColor: resolveRgbVar('--color-border', '51 65 85'),
      dividerColor: resolveRgbVar('--color-border', '51 65 85'),
      bodyColor: resolveRgbVar('--color-bg', '2 6 23'),
      cardColor: resolveRgbVar('--color-panel', '15 23 42'),
      modalColor: resolveRgbVar('--color-panel', '15 23 42'),
      popoverColor: resolveRgbVar('--color-panel', '15 23 42'),
      inputColor: resolveRgbVar('--color-panel-muted', '30 41 59'),
      actionColor: resolveRgbVar('--color-panel-muted', '30 41 59'),
      actionColorHover: resolveRgbVar('--color-panel-muted', '30 41 59'),
      actionColorPressed: resolveRgbVar('--color-panel-muted', '30 41 59'),
    },
    Tabs: {
      tabFontSizeSmall: '12px',
      tabHeightSmall: '24px',
      tabPaddingSmall: '0 10px',
      cardPaddingSmall: '0 4px',
      cardGapSmall: '4px',
    },
  };
});

let streamHandle: ConsoleStreamHandle | null = null;
const lastPendingCounts = ref<Record<string, number>>({});

const pendingTotal = computed(() => targets.value.reduce((sum, target) => sum + target.pending_count, 0));
const selectedTarget = computed(() => targets.value.find((target) => target.name === selectedTargetName.value) ?? null);
const selectedSnapshot = computed(() => {
  if (!selectedTargetName.value) {
    return null;
  }
  return snapshots.value[selectedTargetName.value] ?? null;
});

const {
  activeTerminalTabId,
  closeSelectedTerminal,
  handleActivateTerminalTab,
  handleAddTerminalTab,
  handleCloseTerminalTab,
  openSelectedTerminal,
  selectedTerminalEntry,
  selectedTerminalOpen,
  terminalEntries,
} = useTerminalState({ selectedTargetName, targets });

function showNotification(message: string, count?: number) {
  notification.value = { message, count };
  notificationToken.value += 1;
}

function reportUiError(context: string, err?: unknown) {
  const detail = err ? `: ${String(err)}` : '';
  void logUiEvent(`${context}${detail}`);
}

const { aiRiskMap, enqueueAiTask, processAiQueue, scheduleAiForSnapshot } = useAiRiskQueue({
  settings,
  onError: reportUiError,
});

function scheduleAiForAllTargets() {
  if (!settings.value.ai.enabled) {
    return;
  }
  targets.value.forEach((target) => {
    if (target.pending_count > 0) {
      void refreshSnapshot(target.name);
    }
  });
}

function updateTargets(list: TargetInfo[]) {
  targets.value = list;
  if (!selectedTargetName.value && list.length > 0) {
    selectedTargetName.value = list[0].name;
  }
  list.forEach((target) => {
    const previous = lastPendingCounts.value[target.name] ?? 0;
    if (settings.value.ai.enabled) {
      const hasSnapshot = Boolean(snapshots.value[target.name]);
      if (target.pending_count > 0 && (target.pending_count > previous || !hasSnapshot)) {
        void refreshSnapshot(target.name);
      }
    }
    lastPendingCounts.value[target.name] = target.pending_count;
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

  if (settings.value.ai.enabled && target.pending_count > previous) {
    void refreshSnapshot(target.name);
  } else if (selectedTargetName.value === target.name) {
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
    reportUiError('fetch targets failed', err);
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
    scheduleAiForSnapshot(name, snapshot);
  } catch (err) {
    reportUiError(`fetch snapshot failed target=${name}`, err);
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
    reportUiError('approve command failed', err);
  }
}

async function deny(id: string) {
  if (!selectedTargetName.value) return;
  try {
    await denyCommand(selectedTargetName.value, id);
  } catch (err) {
    showNotification('拒绝失败，请检查 console 服务');
    reportUiError('deny command failed', err);
  }
}

function refreshAiRisk(payload: { target: string; id: string }) {
  const snapshot = snapshots.value[payload.target];
  const request = snapshot?.queue.find((item) => item.id === payload.id);
  if (request) {
    enqueueAiTask(payload.target, request);
    return;
  }
  void refreshSnapshot(payload.target);
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

watch(
  () => settings.value.ai,
  (value, previous) => {
    if (value.enabled && (!previous?.enabled || previous?.apiKey !== value.apiKey)) {
      scheduleAiForAllTargets();
    }
    processAiQueue();
  },
  { deep: true }
);

watch(
  () => settings.value.theme,
  (mode) => {
    applyThemeMode(mode);
  },
  { immediate: true }
);
</script>

<template>
  <n-config-provider :theme="naiveTheme" :theme-overrides="naiveThemeOverrides">
    <n-notification-provider>
      <NotificationBridge :payload="notification" :token="notificationToken" />
      <div class="flex h-screen w-screen bg-surface text-foreground overflow-hidden pt-7">
        <div
          class="fixed top-0 left-0 right-0 h-7 z-30"
          data-tauri-drag-region
          @mousedown="handleTitleDrag"
        ></div>

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
            <n-button
              size="small"
              quaternary
              circle
              title="AI 助手"
              @click="isChatOpen = true"
            >
              <template #icon>
                <n-icon :component="ChatbubblesOutline" />
              </template>
            </n-button>
            <span
              v-if="connectionState === 'disconnected'"
              class="text-xs px-2 py-1 rounded border bg-danger/20 text-danger border-danger/30"
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
            :terminal-open="selectedTerminalOpen"
            :ai-risk-map="aiRiskMap"
            :ai-enabled="settings.ai.enabled"
            @approve="approve"
            @deny="deny"
            @refresh-risk="refreshAiRisk"
            @open-terminal="openSelectedTerminal"
            @close-terminal="closeSelectedTerminal"
          >
            <template #terminal>
              <div class="flex flex-col min-h-0 h-full">
              <div v-if="selectedTerminalEntry" class="pt-1 pb-0 bg-surface">
                  <n-tabs
                    :value="activeTerminalTabId"
                    type="card"
                    size="small"
                    addable
                    closable
                    class="min-w-0 terminal-tabs"
                    @add="handleAddTerminalTab"
                    @close="handleCloseTerminalTab"
                    @update:value="handleActivateTerminalTab"
                  >
                    <n-tab-pane
                      v-for="tab in selectedTerminalEntry.state.tabs"
                      :key="tab.id"
                      :name="tab.id"
                      :tab="tab.label"
                      closable
                    />
                  </n-tabs>
                </div>
                <div class="flex-1 min-h-0 relative">
                  <template v-for="entry in terminalEntries" :key="entry.target.name">
                    <TerminalPanel
                      v-for="tab in entry.state.tabs"
                      :key="tab.id"
                      :target="entry.target"
                      :theme="resolvedTheme"
                      :visible="
                        selectedTerminalOpen &&
                        selectedTargetName === entry.target.name &&
                        entry.state.activeId === tab.id
                      "
                      v-show="
                        selectedTerminalOpen &&
                        selectedTargetName === entry.target.name &&
                        entry.state.activeId === tab.id
                      "
                    />
                  </template>
                </div>
              </div>
            </template>
          </TargetView>
          <div v-else class="flex-1 flex items-center justify-center text-foreground-muted">
            请选择目标开始操作。
          </div>
        </div>
        </div>

        <SettingsModal
          :is-open="isSettingsOpen"
          :settings="settings"
          :resolved-theme="resolvedTheme"
          @close="isSettingsOpen = false"
          @save="handleSettingsSave"
        />

        <ChatDrawer
          v-model="isChatOpen"
          title="AI 助手"
          greeting="你好，我是 AI 助手。请描述你想要完成的任务。"
        />
      </div>
    </n-notification-provider>
  </n-config-provider>
</template>
