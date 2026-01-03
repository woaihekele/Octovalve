<script setup lang="ts">
import { computed, inject, onBeforeUnmount, onMounted, ref, watch } from 'vue';
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
import { useChatStore } from './chat';
import { storeToRefs } from 'pinia';
import type { AuthMethod } from './chat/services/acpService';
import { matchesShortcut } from './shortcuts';
import ConsoleChatPane from './components/ConsoleChatPane.vue';
import ConsoleLeftPane from './components/ConsoleLeftPane.vue';
import SettingsModal from './components/SettingsModal.vue';
import NotificationBridge from './components/NotificationBridge.vue';
import { loadSettings, saveSettings } from './settings';
import type { AppSettings, ConsoleEvent, ServiceSnapshot, TargetInfo } from './types';
import { useAiRiskQueue } from './composables/useAiRiskQueue';
import { useTerminalState } from './composables/useTerminalState';
import type { ResolvedTheme } from './theme';
import { APPLY_THEME_MODE, RESOLVED_THEME } from './appContext';

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
const applyThemeMode = inject(APPLY_THEME_MODE, () => {});
const resolvedTheme = inject(RESOLVED_THEME, ref<ResolvedTheme>('dark'));
const isChatHistoryOpen = ref(false);

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

// Chat store integration
const chatStore = useChatStore();
const { messages: chatMessages, isStreaming: chatIsStreaming, isConnected: chatIsConnected, providerInitialized } = storeToRefs(chatStore);

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

// Re-initialize when settings change
watch(() => settings.value.chat.provider, async (newProvider, oldProvider) => {
  if (newProvider !== oldProvider) {
    console.log(`[Chat] Switching provider from ${oldProvider} to ${newProvider}`);
    try {
      // Stop current provider
      if (oldProvider === 'openai') {
        console.log('[Chat] Stopping OpenAI...');
        await chatStore.stopOpenai();
      } else {
        console.log('[Chat] Stopping ACP...');
        await chatStore.stopAcp();
      }
      console.log('[Chat] Previous provider stopped');
      // Initialize new provider
      console.log('[Chat] Initializing new provider...');
      await initChatProvider();
      console.log('[Chat] New provider initialized');
    } catch (e) {
      console.error('[Chat] Provider switch failed:', e);
    }
  }
});

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

function handleChatCancel() {
  if (chatStore.provider === 'acp' && chatStore.acpInitialized) {
    chatStore.cancelAcp();
  } else if (chatStore.provider === 'openai' && chatStore.openaiInitialized) {
    chatStore.cancelOpenai();
  }
  chatStore.setStreaming(false);
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
  if (settings.value.chat.provider !== newProvider) {
    settings.value.chat.provider = newProvider;
    saveSettings(settings.value);
  }
}

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
  if (event.defaultPrevented) {
    return;
  }
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
  <NotificationBridge :payload="notification" :token="notificationToken" />
  <div class="flex h-screen w-screen bg-surface text-foreground overflow-hidden pt-7">
    <div
      class="fixed top-0 left-0 right-0 h-7 z-[4000] pointer-events-auto"
      data-tauri-drag-region
    ></div>

    <ConsoleLeftPane
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
      :provider="settings.chat.provider"
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
      @close="isSettingsOpen = false"
      @save="handleSettingsSave"
    />

  </div>
</template>
