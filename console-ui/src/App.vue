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
import { NButton, NConfigProvider, NNotificationProvider, NTabPane, NTabs, darkTheme } from 'naive-ui';
import { ChatPanel, useChatStore } from './chat';
import { storeToRefs } from 'pinia';
import type { AuthMethod } from './chat/services/acpService';
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

const DARCULA_PRIMARY = 'rgb(54, 88, 128)';
const DARCULA_PRIMARY_HOVER = 'rgb(62, 100, 145)';
const DARCULA_PRIMARY_PRESSED = 'rgb(43, 70, 103)';

const naiveThemeOverrides = computed(() => {
  void resolvedTheme.value;
  const isDarcula = resolvedTheme.value === 'darcula';
  return {
    common: {
      primaryColor: isDarcula ? DARCULA_PRIMARY : resolveRgbVar('--color-accent', '99 102 241'),
      primaryColorHover: isDarcula ? DARCULA_PRIMARY_HOVER : resolveRgbVar('--color-accent', '99 102 241'),
      primaryColorPressed: isDarcula ? DARCULA_PRIMARY_PRESSED : resolveRgbVar('--color-accent-soft', '67 56 202'),
      primaryColorSuppl: isDarcula ? DARCULA_PRIMARY_PRESSED : resolveRgbVar('--color-accent-soft', '67 56 202'),
      successColor: resolveRgbVar('--color-success', '52 211 153'),
      warningColor: resolveRgbVar('--color-warning', '251 191 36'),
      errorColor: resolveRgbVar('--color-danger', '244 63 94'),
      textColorBase: resolveRgbVar('--color-text', '226 232 240'),
      textColor1: resolveRgbVar('--color-text', '226 232 240'),
      textColor2: resolveRgbVar('--color-text-muted', '100 116 139'),
      textColor3: resolveRgbVar('--color-text-muted', '100 116 139'),
      placeholderColor: isDarcula ? 'rgb(170, 170, 170)' : resolveRgbVar('--color-text-muted', '100 116 139'),
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
    Switch: {
      railColorActive: isDarcula ? DARCULA_PRIMARY : undefined,
    },
    Button: {
      colorPrimary: isDarcula ? DARCULA_PRIMARY : undefined,
      colorHoverPrimary: isDarcula ? DARCULA_PRIMARY_HOVER : undefined,
      colorPressedPrimary: isDarcula ? DARCULA_PRIMARY_PRESSED : undefined,
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
      await chatStore.initializeAcp(chatConfig.acp.path || '.');
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

        <div class="flex-1 flex min-w-0 min-h-0 overflow-hidden">
          <div class="flex-1 flex flex-col min-w-0 min-h-0 relative">
            <div class="absolute top-4 right-4 z-20 flex items-center gap-3">
              <span
                v-if="connectionState === 'disconnected'"
                class="text-xs px-2 py-1 rounded border bg-danger/20 text-danger border-danger/30"
              >
                console 异常，请重启
              </span>

              <!-- Chat toggle button -->
              <button
                v-if="!selectedTarget"
                class="p-2 rounded border transition-colors bg-panel/60 text-foreground border-border hover:border-accent/40"
                @click="isChatOpen = !isChatOpen"
                :aria-label="isChatOpen ? '收起 AI 助手' : '展开 AI 助手'"
                :title="isChatOpen ? '收起 AI 助手' : '展开 AI 助手'"
              >
                <svg
                  class="h-4 w-4"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.6"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <path v-if="!isChatOpen" d="M10 6l6 6-6 6" />
                  <path v-else d="M14 6l-6 6 6 6" />
                </svg>
              </button>
            </div>

          <div class="flex-1 min-h-0">
            <TargetView
              v-if="selectedTarget"
              :target="selectedTarget"
              :snapshot="selectedSnapshot"
              :settings="settings"
              :pending-jump-token="pendingJumpToken"
              :terminal-open="selectedTerminalOpen"
              :chat-open="isChatOpen"
              :ai-risk-map="aiRiskMap"
              :ai-enabled="settings.ai.enabled"
              @approve="approve"
              @deny="deny"
              @refresh-risk="refreshAiRisk"
              @open-terminal="openSelectedTerminal"
              @close-terminal="closeSelectedTerminal"
              @toggle-chat="isChatOpen = !isChatOpen"
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

          <ChatPanel
            :is-open="isChatOpen"
            :messages="chatMessages"
            :is-streaming="chatIsStreaming"
            :is-connected="chatIsConnected"
            :provider="settings.chat.provider"
            title="AI 助手"
            greeting="你好，我是 AI 助手"
            @send="handleChatSend"
            @cancel="handleChatCancel"
            @new-session="handleChatNewSession"
            @clear="handleChatClear"
            @change-provider="handleChangeProvider"
          />
        </div>

        <SettingsModal
          :is-open="isSettingsOpen"
          :settings="settings"
          :resolved-theme="resolvedTheme"
          @close="isSettingsOpen = false"
          @save="handleSettingsSave"
        />
      </div>
    </n-notification-provider>
  </n-config-provider>
</template>
