<script setup lang="ts">
import { computed, inject, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { NButton, NCard, NModal, NSelect } from 'naive-ui';
import { isTauri } from '@tauri-apps/api/core';
import {
  approveCommand,
  denyCommand,
  fetchSnapshot,
  fetchTargets,
  getProxyConfigStatus,
  listProfiles,
  logUiEvent,
  openConsoleStream,
  restartConsole,
  selectProfile,
  validateStartupConfig,
  type ConsoleConnectionStatus,
  type ConsoleStreamHandle,
} from '../services/api';
import { useChatStore } from '../domain/chat';
import { storeToRefs } from 'pinia';
import type { AuthMethod } from '../domain/chat/services/acpService';
import { matchesShortcut } from '../shared/shortcuts';
import ConsoleChatPane from '../ui/components/ConsoleChatPane.vue';
import ConsoleLeftPane from '../ui/components/ConsoleLeftPane.vue';
import SettingsModal from '../ui/components/SettingsModal.vue';
import NotificationBridge from '../ui/components/NotificationBridge.vue';
import { loadSettings, saveSettings } from '../services/settings';
import type { AppSettings, ConsoleEvent, ProfileSummary, ServiceSnapshot, TargetInfo } from '../shared/types';
import { useAiRiskQueue } from '../composables/useAiRiskQueue';
import { useTerminalState } from '../composables/useTerminalState';
import type { ResolvedTheme } from '../shared/theme';
import { APPLY_THEME_MODE, RESOLVED_THEME } from './appContext';

const targets = ref<TargetInfo[]>([]);
const snapshots = ref<Record<string, ServiceSnapshot>>({});
const selectedTargetName = ref<string | null>(null);
const settings = ref(loadSettings());
const isSettingsOpen = ref(false);
const isChatOpen = ref(false);
const notification = ref<{ message: string; count?: number; target?: string } | null>(null);
const notificationToken = ref(0);
const connectionState = ref<'connected' | 'connecting' | 'disconnected'>('connecting');
const snapshotLoading = ref<Record<string, boolean>>({});
const snapshotRefreshPending = ref<Record<string, boolean>>({});
const pendingJumpToken = ref(0);
const applyThemeMode = inject(APPLY_THEME_MODE, () => {});
const resolvedTheme = inject(RESOLVED_THEME, ref<ResolvedTheme>('dark'));
const isChatHistoryOpen = ref(false);
const startupProfileOpen = ref(false);
const startupProfiles = ref<ProfileSummary[]>([]);
const startupSelectedProfile = ref<string | null>(null);
const startupBusy = ref(false);
const startupStatusMessage = ref('');
const startupError = ref('');
const booting = ref(false);
const hasConnected = ref(false);
const focusConfigToken = ref(0);
type ConsoleLeftPaneExpose = {
  focusActiveTerminal: () => void;
  blurActiveTerminal: () => void;
  isActiveTerminalFocused: () => boolean;
};
const leftPaneRef = ref<ConsoleLeftPaneExpose | null>(null);
const lastNonTerminalFocus = ref<HTMLElement | null>(null);

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
const startupProfileOptions = computed(() =>
  startupProfiles.value.map((profile) => ({ label: profile.name, value: profile.name }))
);
const consoleBanner = computed<{ kind: 'error' | 'info'; message: string } | null>(() => {
  if (booting.value) {
    return { kind: 'info', message: 'console 启动中...' };
  }
  if (!hasConnected.value && connectionState.value === 'connecting') {
    return { kind: 'info', message: 'console 连接中...' };
  }
  if (hasConnected.value && connectionState.value === 'disconnected') {
    return { kind: 'error', message: 'console 异常，请重启' };
  }
  return null;
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

// Chat store integration
const chatStore = useChatStore();
const {
  messages: chatMessages,
  isStreaming: chatIsStreaming,
  isConnected: chatIsConnected,
  providerInitialized,
  provider: chatProvider,
} = storeToRefs(chatStore);
const providerSwitchConfirmOpen = ref(false);
const pendingProvider = ref<'acp' | 'openai' | null>(null);
const providerSwitching = ref(false);
const pendingProviderLabel = computed(() => {
  if (pendingProvider.value === 'acp') {
    return 'ACP';
  }
  if (pendingProvider.value === 'openai') {
    return 'API';
  }
  return '未知';
});

// Initialize chat provider based on settings
async function initChatProvider() {
  const chatConfig = settings.value.chat;
  console.log('[initChatProvider] config:', chatConfig.provider);
  
  try {
    if (chatConfig.provider === 'openai') {
      await chatStore.initializeOpenai(chatConfig.openai);
    } else {
      // ACP provider
      console.log('[initChatProvider] calling initializeAcp...');
      await chatStore.initializeAcp('.', chatConfig.acp.path);
      console.log('[initChatProvider] initializeAcp done, providerInitialized:', providerInitialized.value);
      
      // Authentication is optional - don't fail if it's not available
      if ((chatStore.authMethods as AuthMethod[]).some((m) => m.id === 'openai-api-key')) {
        try {
          await chatStore.authenticateAcp('openai-api-key');
          console.log('[initChatProvider] authenticateAcp done');
        } catch (authErr) {
          console.warn('[initChatProvider] authenticateAcp failed (optional):', authErr);
        }
      }
    }
    console.log('[initChatProvider] final providerInitialized:', providerInitialized.value);
  } catch (e) {
    console.warn('Chat provider initialization failed:', e);
  }
}

// Call init after a short delay to let Tauri initialize
setTimeout(initChatProvider, 500);

async function handleChatSend(content: string) {
  console.log('[handleChatSend] providerInitialized:', providerInitialized.value, 'provider:', chatStore.provider);
  if (providerInitialized.value) {
    try {
      await chatStore.sendMessage(content);
    } catch (e) {
      showNotification(`Chat error: ${e}`);
    }
  } else {
    // Fallback to simulated response
    chatStore.addMessage({
      type: 'say',
      say: 'text',
      role: 'user',
      content,
      status: 'complete',
    });

    const assistantMsg = chatStore.addMessage({
      type: 'say',
      say: 'text',
      role: 'assistant',
      content: '',
      status: 'streaming',
      partial: true,
    });

    chatStore.setStreaming(true);

    const response = `收到你的消息: "${content}"\n\nACP 未初始化，这是模拟响应。请确保 codex-acp 已安装并配置。`;
    for (let i = 0; i < response.length; i++) {
      chatStore.appendToMessage(assistantMsg.id, response[i]);
      await new Promise((r) => setTimeout(r, 15));
    }

    chatStore.updateMessage(assistantMsg.id, { status: 'complete', partial: false });
    chatStore.setStreaming(false);
  }
}

async function cancelActiveChat() {
  if (chatStore.provider === 'acp' && chatStore.acpInitialized) {
    await chatStore.cancelAcp();
  } else if (chatStore.provider === 'openai' && chatStore.openaiInitialized) {
    await chatStore.cancelOpenai();
  }
  chatStore.setStreaming(false);
}

async function handleChatCancel() {
  await cancelActiveChat();
}

function handleChatNewSession() {
  chatStore.createSession();
}

function handleChatClear() {
  chatStore.clearMessages();
}

function handleChatShowHistory() {
  isChatHistoryOpen.value = true;
}

const openaiSessions = computed(() => chatStore.sessions.filter((s) => s.provider === 'openai'));

function handleChangeProvider(newProvider: 'acp' | 'openai') {
  if (chatProvider.value === newProvider) {
    return;
  }
  pendingProvider.value = newProvider;
  providerSwitchConfirmOpen.value = true;
}

function cancelProviderSwitch() {
  providerSwitchConfirmOpen.value = false;
  pendingProvider.value = null;
}

async function stopActiveProvider() {
  if (chatStore.provider === 'openai') {
    await chatStore.stopOpenai();
    return;
  }
  await chatStore.stopAcp();
}

async function confirmProviderSwitch() {
  if (!pendingProvider.value || providerSwitching.value) {
    return;
  }
  const targetProvider = pendingProvider.value;
  providerSwitching.value = true;
  try {
    if (chatIsStreaming.value) {
      await cancelActiveChat();
    }
    await stopActiveProvider();
    settings.value.chat.provider = targetProvider;
    saveSettings(settings.value);
    await initChatProvider();
    chatStore.createSession();
  } catch (e) {
    console.error('[Chat] Provider switch failed:', e);
    showNotification(`切换失败：${String(e)}`);
  } finally {
    providerSwitching.value = false;
    providerSwitchConfirmOpen.value = false;
    pendingProvider.value = null;
  }
}

function showNotification(message: string, count?: number, target?: string) {
  notification.value = { message, count, target };
  notificationToken.value += 1;
}

function reportUiError(context: string, err?: unknown) {
  const detail = err ? `: ${String(err)}` : '';
  void logUiEvent(`${context}${detail}`);
}

function openSettingsForConfig() {
  isSettingsOpen.value = true;
  focusConfigToken.value += 1;
}

async function startConsoleSession() {
  await refreshTargets();
  void connectWebSocket();
  void logUiEvent(`origin=${window.location.origin} secure=${window.isSecureContext}`);
}

async function loadStartupProfiles() {
  if (!isTauri()) {
    return;
  }
  startupBusy.value = true;
  startupError.value = '';
  startupStatusMessage.value = '正在加载环境配置...';
  try {
    const data = await listProfiles();
    startupProfiles.value = data.profiles;
    startupSelectedProfile.value = data.current || data.profiles[0]?.name || null;
    startupProfileOpen.value = true;
    connectionState.value = 'disconnected';
    startupStatusMessage.value = '';
  } catch (err) {
    const message = `加载环境失败：${String(err)}`;
    startupError.value = message;
    showNotification(message);
    reportUiError('load profiles failed', err);
    startupProfileOpen.value = true;
  } finally {
    startupBusy.value = false;
  }
}

async function applyStartupProfile() {
  if (startupBusy.value) {
    return;
  }
  if (!startupSelectedProfile.value) {
    startupError.value = '请选择要启动的环境。';
    return;
  }
  startupBusy.value = true;
  startupError.value = '';
  booting.value = true;
  hasConnected.value = false;
  connectionState.value = 'connecting';
  startupStatusMessage.value = `正在应用环境 ${startupSelectedProfile.value}...`;
  try {
    await selectProfile(startupSelectedProfile.value);
    const status = await getProxyConfigStatus();
    if (!status.present) {
      const message = `未找到 ${status.path}，请参考 ${status.example_path} 创建并修改后重试`;
      startupError.value = message;
      showNotification(message);
      connectionState.value = 'disconnected';
      booting.value = false;
      return;
    }
    startupStatusMessage.value = '正在校验配置...';
    const check = await validateStartupConfig();
    if (!check.ok) {
      const message = `配置检查失败：\n- ${check.errors.join('\n- ')}`;
      startupError.value = message;
      showNotification('配置检查失败，请查看详情');
      reportUiError('startup config invalid', check.errors.join(' | '));
      connectionState.value = 'disconnected';
      booting.value = false;
      if (check.needs_setup) {
        openSettingsForConfig();
      }
      return;
    }
    startupStatusMessage.value = '正在启动 console...';
    await restartConsole();
    startupProfileOpen.value = false;
    startupStatusMessage.value = '';
    await startConsoleSession();
  } catch (err) {
    const message = `环境启动失败：${String(err)}`;
    startupError.value = message;
    showNotification(message);
    reportUiError('startup profile failed', err);
    connectionState.value = 'disconnected';
    booting.value = false;
  } finally {
    startupBusy.value = false;
  }
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

function shouldRefreshSnapshotFromTargetsSnapshot(target: TargetInfo, previousPending: number) {
  const hasSnapshot = Boolean(snapshots.value[target.name]);
  const isSelected = selectedTargetName.value === target.name;
  const pendingChanged = target.pending_count !== previousPending;
  if (settings.value.ai.enabled && target.pending_count > 0 && (target.pending_count > previousPending || !hasSnapshot)) {
    return true;
  }
  if (isSelected && (pendingChanged || !hasSnapshot)) {
    return true;
  }
  return false;
}

function shouldRefreshSnapshotFromTargetUpdate(target: TargetInfo, previousPending: number) {
  const hasSnapshot = Boolean(snapshots.value[target.name]);
  const isSelected = selectedTargetName.value === target.name;
  if (isSelected) {
    return true;
  }
  if (settings.value.ai.enabled && target.pending_count > 0 && (target.pending_count > previousPending || !hasSnapshot)) {
    return true;
  }
  return false;
}

function updateTargets(list: TargetInfo[]) {
  targets.value = list;
  if (!selectedTargetName.value && list.length > 0) {
    selectedTargetName.value = list[0].name;
  }
  list.forEach((target) => {
    const previous = lastPendingCounts.value[target.name] ?? 0;
    if (shouldRefreshSnapshotFromTargetsSnapshot(target, previous)) {
      void refreshSnapshot(target.name);
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
    showNotification(`收到 ${target.name} 的新请求`, target.pending_count, target.name);
  }
  if (shouldRefreshSnapshotFromTargetUpdate(target, previous)) {
    void refreshSnapshot(target.name);
  }
  lastPendingCounts.value[target.name] = target.pending_count;
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
        hasConnected.value = true;
        booting.value = false;
        void logUiEvent('ws connected');
      } else if (status === 'disconnected') {
        void logUiEvent('ws closed');
      }
    });
  } catch (err) {
    connectionState.value = 'disconnected';
    booting.value = false;
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
    snapshotRefreshPending.value[name] = true;
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
    if (snapshotRefreshPending.value[name]) {
      snapshotRefreshPending.value[name] = false;
      void refreshSnapshot(name);
    }
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

function handleNotificationJump(targetName: string) {
  const target = targets.value.find((item) => item.name === targetName);
  if (!target || target.pending_count === 0) {
    return;
  }
  selectedTargetName.value = target.name;
  pendingJumpToken.value += 1;
}

function isCtrlBackquote(event: KeyboardEvent) {
  return event.ctrlKey && !event.altKey && !event.metaKey && event.code === 'Backquote';
}

function isCtrlArrow(event: KeyboardEvent, direction: 'prev' | 'next') {
  const code = direction === 'prev' ? 'ArrowLeft' : 'ArrowRight';
  return event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey && event.code === code;
}

function isEditableTarget(target: EventTarget | null) {
  if (!target) {
    return false;
  }
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
    return true;
  }
  return target instanceof HTMLElement && target.isContentEditable;
}

function toggleTerminalFocus(): boolean {
  if (!selectedTerminalOpen.value) {
    return false;
  }
  const pane = leftPaneRef.value;
  if (!pane) {
    return false;
  }
  if (pane.isActiveTerminalFocused()) {
    pane.blurActiveTerminal();
    const last = lastNonTerminalFocus.value;
    if (last && last.isConnected) {
      last.focus();
      return true;
    }
    const active = document.activeElement;
    if (active instanceof HTMLElement) {
      active.blur();
    }
    return true;
  }
  const active = document.activeElement;
  if (active instanceof HTMLElement) {
    lastNonTerminalFocus.value = active;
  } else {
    lastNonTerminalFocus.value = null;
  }
  pane.focusActiveTerminal();
  return true;
}

function switchTerminalSession(direction: 'prev' | 'next'): boolean {
  const entry = selectedTerminalEntry.value;
  if (!entry) {
    return false;
  }
  const tabs = entry.state.tabs;
  if (tabs.length <= 1) {
    return false;
  }
  const activeId = entry.state.activeId ?? tabs[0]?.id;
  if (!activeId) {
    return false;
  }
  const currentIndex = tabs.findIndex((tab) => tab.id === activeId);
  if (currentIndex < 0) {
    return false;
  }
  const nextIndex = direction === 'prev' ? currentIndex - 1 : currentIndex + 1;
  if (nextIndex < 0 || nextIndex >= tabs.length) {
    return false;
  }
  handleActivateTerminalTab(tabs[nextIndex].id);
  return true;
}

function handleGlobalKey(event: KeyboardEvent) {
  if (isCtrlBackquote(event)) {
    if (isSettingsOpen.value) {
      return;
    }
    if (toggleTerminalFocus()) {
      event.preventDefault();
    }
    return;
  }
  if (event.defaultPrevented) {
    return;
  }
  if (matchesShortcut(event, settings.value.shortcuts.openSettings)) {
    event.preventDefault();
    isSettingsOpen.value = true;
    return;
  }
  if (isEditableTarget(event.target)) {
    return;
  }
  if (isSettingsOpen.value) {
    return;
  }
  if (isCtrlArrow(event, 'prev')) {
    if (switchTerminalSession('prev')) {
      event.preventDefault();
    }
    return;
  }
  if (isCtrlArrow(event, 'next')) {
    if (switchTerminalSession('next')) {
      event.preventDefault();
    }
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

onMounted(async () => {
  window.addEventListener('keydown', handleGlobalKey);
  if (isTauri()) {
    await loadStartupProfiles();
    return;
  }
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
  await startConsoleSession();
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
  <NotificationBridge :payload="notification" :token="notificationToken" @jump-pending="handleNotificationJump" />
  <n-modal v-model:show="startupProfileOpen" :mask-closable="false" :close-on-esc="false">
    <n-card size="small" class="w-[26rem]" :bordered="true">
      <template #header>选择启动环境</template>
      <div class="space-y-3">
        <div class="text-xs text-foreground-muted">
          每次启动需要选择要使用的环境配置。
        </div>
        <NSelect
          :value="startupSelectedProfile"
          :options="startupProfileOptions"
          size="small"
          :disabled="startupBusy || startupProfileOptions.length === 0"
          placeholder="请选择环境"
          to="body"
          @update:value="(value) => { startupSelectedProfile = value as string; startupError = ''; }"
        />
        <div v-if="startupError" class="text-xs text-danger whitespace-pre-wrap">
          {{ startupError }}
        </div>
        <div v-else-if="startupStatusMessage" class="text-xs text-foreground-muted">
          {{ startupStatusMessage }}
        </div>
        <div class="flex items-center justify-end gap-2 pt-1">
          <n-button size="small" :disabled="startupBusy" @click="isSettingsOpen = true">打开设置</n-button>
          <n-button
            size="small"
            type="primary"
            :disabled="startupBusy || !startupSelectedProfile"
            @click="applyStartupProfile"
          >
            {{ startupBusy ? '启动中…' : '开始' }}
          </n-button>
        </div>
      </div>
    </n-card>
  </n-modal>
  <div class="flex h-screen w-screen bg-surface text-foreground overflow-hidden pt-7">
    <div
      class="fixed top-0 left-0 right-0 h-7 z-[4000] pointer-events-auto"
      data-tauri-drag-region
    ></div>

    <ConsoleLeftPane
      ref="leftPaneRef"
      :targets="targets"
      :selected-target-name="selectedTargetName"
      :pending-total="pendingTotal"
      :connection-state="connectionState"
      :selected-target="selectedTarget"
      :selected-snapshot="selectedSnapshot"
      :settings="settings"
      :pending-jump-token="pendingJumpToken"
      :selected-terminal-open="selectedTerminalOpen"
      :is-chat-open="isChatOpen"
      :ai-risk-map="aiRiskMap"
      :ai-enabled="settings.ai.enabled"
      :console-banner="consoleBanner"
      :selected-terminal-entry="selectedTerminalEntry"
      :active-terminal-tab-id="activeTerminalTabId"
      :terminal-entries="terminalEntries"
      :resolved-theme="resolvedTheme"
      @select-target="selectedTargetName = $event"
      @open-settings="isSettingsOpen = true"
      @toggle-chat="isChatOpen = !isChatOpen"
      @approve="approve"
      @deny="deny"
      @refresh-risk="refreshAiRisk"
      @open-terminal="openSelectedTerminal"
      @close-terminal="closeSelectedTerminal"
      @terminal-add="handleAddTerminalTab"
      @terminal-close="handleCloseTerminalTab"
      @terminal-activate="handleActivateTerminalTab"
    />

    <ConsoleChatPane
      :is-chat-open="isChatOpen"
      :messages="chatMessages"
      :is-streaming="chatIsStreaming"
      :is-connected="chatIsConnected"
      :provider="chatProvider"
      :is-history-open="isChatHistoryOpen"
      :openai-sessions="openaiSessions"
      :active-session-id="chatStore.activeSessionId"
      @send="handleChatSend"
      @cancel="handleChatCancel"
      @show-history="handleChatShowHistory"
      @new-session="handleChatNewSession"
      @clear="handleChatClear"
      @change-provider="handleChangeProvider"
      @close-history="isChatHistoryOpen = false"
      @select-session="(id) => { chatStore.setActiveSession(id); isChatHistoryOpen = false; }"
      @delete-session="(id) => chatStore.deleteSessionForProvider(id, 'openai')"
      @clear-sessions="() => chatStore.clearSessionsForProvider('openai')"
    />

    <SettingsModal
      :is-open="isSettingsOpen"
      :settings="settings"
      :resolved-theme="resolvedTheme"
      :focus-config-token="focusConfigToken"
      @close="isSettingsOpen = false"
      @save="handleSettingsSave"
    />

    <n-modal v-model:show="providerSwitchConfirmOpen" :mask-closable="false" :close-on-esc="true">
      <n-card size="small" class="w-[24rem]" :bordered="true">
        <template #header>切换对话 Provider</template>
        <div class="space-y-2 text-sm text-foreground-muted">
          <div>切换到 {{ pendingProviderLabel }} 会创建一个全新的会话，历史会话保留。</div>
          <div>当前正在生成的回复会被强制停止。</div>
        </div>
        <template #footer>
          <div class="flex justify-end gap-2">
            <n-button :disabled="providerSwitching" @click="cancelProviderSwitch">取消</n-button>
            <n-button type="primary" :disabled="providerSwitching" @click="confirmProviderSwitch">确认切换</n-button>
          </div>
        </template>
      </n-card>
    </n-modal>

  </div>
</template>
