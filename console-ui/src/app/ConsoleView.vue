<script setup lang="ts">
import { computed, inject, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { NButton, NCard, NModal, NSelect, NSpin } from 'naive-ui';
import { Terminal, type ITheme } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { isTauri } from '@tauri-apps/api/core';
import {
  approveCommand,
  cancelCommand,
  denyCommand,
  fetchSnapshot,
  fetchTargets,
  getProxyConfigStatus,
  listProfiles,
  logUiEvent,
  openConsoleStream,
  readAppLog,
  readConsoleLog,
  restartConsole,
  setAppLanguage,
  selectProfile,
  validateStartupConfig,
  type ConsoleConnectionStatus,
  type ConsoleStreamHandle,
} from '../services/api';
import { useChatStore } from '../domain/chat';
import { storeToRefs } from 'pinia';
import type { AuthMethod, AcpSessionSummary } from '../domain/chat/services/acpService';
import type { ChatSession, SendMessageOptions } from '../domain/chat/types';
import { matchesShortcut } from '../shared/shortcuts';
import ConsoleChatPane from '../ui/components/ConsoleChatPane.vue';
import ConsoleLeftPane from '../ui/components/ConsoleLeftPane.vue';
import SettingsModal from '../ui/components/SettingsModal.vue';
import NotificationBridge from '../ui/components/NotificationBridge.vue';
import { loadSettings, saveSettings } from '../services/settings';
import { applyUiScale } from '../services/uiScale';
import { getWindowLogicalSize, setWindowMinSize, setWindowSize } from '../services/tauriWindow';
import type { AppLanguage, AppSettings, ConsoleEvent, ProfileSummary, ServiceSnapshot, TargetInfo } from '../shared/types';
import { useAiRiskQueue } from '../composables/useAiRiskQueue';
import { useTerminalState } from '../composables/useTerminalState';
import type { ResolvedTheme } from '../shared/theme';
import { APPLY_THEME_MODE, RESOLVED_THEME } from './appContext';
import {
  CHAT_MAX_WIDTH,
  CHAT_MIN_WIDTH,
  SIDEBAR_MIN_WIDTH,
  SIDEBAR_WIDTH,
  TARGET_MIN_MAIN_WIDTH,
  WINDOW_MIN_HEIGHT,
} from '../ui/layout';

const targets = ref<TargetInfo[]>([]);
const snapshots = ref<Record<string, ServiceSnapshot>>({});
const selectedTargetName = ref<string | null>(null);
const settings = ref(loadSettings());
const { locale, t } = useI18n({ useScope: 'global' });
locale.value = settings.value.language;
const tauriAvailable = isTauri();
const isSettingsOpen = ref(false);
const isChatOpen = ref(false);
const isFileDragging = ref(false);
let dragIdleTimer: number | null = null;
let lastFileDragAt = 0;
const DRAG_IDLE_TIMEOUT = 200;
const notification = ref<{ message: string; count?: number; target?: string; type?: 'success' | 'warning' | 'error' | 'info' } | null>(null);
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
const previewLanguage = ref<AppLanguage | null>(null);
const previewUiScale = ref<number | null>(null);
const previewTerminalScale = ref<number | null>(null);
const STARTUP_SESSION_KEY = 'octovalve.startup.completed';
const quickProfiles = ref<ProfileSummary[]>([]);
const quickProfileCurrent = ref<string | null>(null);
const quickProfileLoading = ref(false);
const quickProfileSwitching = ref(false);
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
const effectiveLanguage = computed(
  () => previewLanguage.value ?? settings.value.language
);
const effectiveUiScale = computed(
  () => previewUiScale.value ?? settings.value.uiScale
);
const effectiveTerminalScale = computed(
  () => previewTerminalScale.value ?? settings.value.terminalScale
);
const DRAG_REGION_BASE_PX = 28;
const dragRegionHeight = computed(() => {
  const scale = Number.isFinite(effectiveUiScale.value) && effectiveUiScale.value > 0
    ? effectiveUiScale.value
    : 1;
  return `${DRAG_REGION_BASE_PX / scale}px`;
});
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
    return { kind: 'info', message: t('console.banner.booting') };
  }
  if (!hasConnected.value && connectionState.value === 'connecting') {
    return { kind: 'info', message: t('console.banner.connecting') };
  }
  if (hasConnected.value && connectionState.value === 'disconnected') {
    return { kind: 'error', message: t('console.banner.disconnected') };
  }
  return null;
});
const windowWidth = ref(typeof window !== 'undefined' ? window.innerWidth : 0);
const isWindowResizing = ref(false);
let windowResizeIdleTimer: number | null = null;
const chatWidthStorageKey = 'console-ui.chat-panel.width';
function readStoredChatWidth() {
  if (typeof window === 'undefined') {
    return CHAT_MIN_WIDTH;
  }
  const raw = window.localStorage.getItem(chatWidthStorageKey);
  if (!raw) {
    return CHAT_MIN_WIDTH;
  }
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed)) {
    return CHAT_MIN_WIDTH;
  }
  return clampChatWidth(parsed);
}
const chatDesiredWidth = ref(readStoredChatWidth());

function clampChatWidth(value: number) {
  return Math.min(CHAT_MAX_WIDTH, Math.max(CHAT_MIN_WIDTH, value));
}

const layoutSizing = computed(() => {
  const chatOpen = isChatOpen.value;
  const scale = Number.isFinite(effectiveUiScale.value) && effectiveUiScale.value > 0
    ? effectiveUiScale.value
    : 1;
  const mainMin = TARGET_MIN_MAIN_WIDTH;
  const minChat = chatOpen ? CHAT_MIN_WIDTH : 0;
  const desiredChat = chatOpen ? clampChatWidth(chatDesiredWidth.value) : 0;
  const width = windowWidth.value || SIDEBAR_WIDTH + TARGET_MIN_MAIN_WIDTH + desiredChat;
  const baseSidebar = SIDEBAR_WIDTH;
  const minSidebar = SIDEBAR_MIN_WIDTH;

  let sidebarWidth = baseSidebar;

  if (chatOpen) {
    const baseTotal = baseSidebar + mainMin + desiredChat;
    if (width < baseTotal) {
      const remaining = width - mainMin;
      sidebarWidth = Math.min(baseSidebar, Math.max(minSidebar, remaining - desiredChat));
      if (remaining - sidebarWidth < minChat) {
        sidebarWidth = Math.max(minSidebar, remaining - minChat);
      }
    }
  } else if (width < baseSidebar + mainMin) {
    const remaining = width - mainMin;
    sidebarWidth = Math.min(baseSidebar, Math.max(minSidebar, remaining));
  }

  const chatMaxWidth = chatOpen
    ? Math.min(CHAT_MAX_WIDTH, width - mainMin - minSidebar)
    : CHAT_MAX_WIDTH;

  return {
    sidebarWidth: Math.max(minSidebar, Math.min(baseSidebar, sidebarWidth)),
    chatMaxWidth: Math.max(minChat, chatMaxWidth),
    minWindowWidth: (mainMin + minSidebar + minChat) * scale,
    minWindowHeight: WINDOW_MIN_HEIGHT * scale,
  };
});

const chatMaxWidth = computed(() => layoutSizing.value.chatMaxWidth);

async function syncWindowMinSize() {
  if (!tauriAvailable) {
    return;
  }
  const minWidth = layoutSizing.value.minWindowWidth;
  const minHeight = layoutSizing.value.minWindowHeight;
  await setWindowMinSize(minWidth, minHeight);
  const logicalSize = await getWindowLogicalSize();
  if (!logicalSize) {
    return;
  }
  const nextWidth = Math.max(logicalSize.width, minWidth);
  const nextHeight = Math.max(logicalSize.height, minHeight);
  if (nextWidth !== logicalSize.width || nextHeight !== logicalSize.height) {
    await setWindowSize(nextWidth, nextHeight);
  }
}

watch(
  () => [layoutSizing.value.minWindowWidth, layoutSizing.value.minWindowHeight],
  () => {
    void syncWindowMinSize();
  }
);

function handleChatWidthChange(width: number) {
  chatDesiredWidth.value = width;
}

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
  planEntries: chatPlanEntries,
  isStreaming: chatIsStreaming,
  isConnected: chatIsConnected,
  providerInitialized,
  provider: chatProvider,
  providerSupportsImages,
  acpHistorySummaries,
  acpHistoryLoading,
} = storeToRefs(chatStore);
const providerSwitchConfirmOpen = ref(false);
const pendingProvider = ref<'acp' | 'openai' | null>(null);
const providerSwitching = ref(false);
const acpRestarting = ref(false);
const chatInputLocked = computed(() => providerSwitching.value || acpRestarting.value);
const showChatDropHint = computed(() => isFileDragging.value);
const pendingProviderLabel = computed(() => {
  if (pendingProvider.value === 'acp') {
    return t('chat.provider.acpLabel');
  }
  if (pendingProvider.value === 'openai') {
    return t('chat.provider.openaiLabel');
  }
  return t('common.unknown');
});
function handleWindowResize() {
  if (typeof window === 'undefined') {
    return;
  }
  windowWidth.value = window.innerWidth;
  isWindowResizing.value = true;
  if (windowResizeIdleTimer !== null) {
    window.clearTimeout(windowResizeIdleTimer);
  }
  windowResizeIdleTimer = window.setTimeout(() => {
    isWindowResizing.value = false;
    windowResizeIdleTimer = null;
  }, 140);
}
const switchLogOpen = ref(false);
const switchLogInProgress = ref(false);
const switchLogStatusMessage = ref('');
const switchLogHasOutput = ref(false);
const switchLogOffset = ref(0);
const switchLogTerminalRef = ref<HTMLDivElement | null>(null);
const SWITCH_LOG_BASE_FONT_SIZE = 12;
let switchLogPollTimer: number | null = null;
let switchLogTerminal: Terminal | null = null;
let switchLogFitAddon: FitAddon | null = null;
let switchLogResizeObserver: ResizeObserver | null = null;
const switchLogContext = ref<'console' | 'acp'>('console');
const switchLogTitle = computed(() =>
  switchLogContext.value === 'acp'
    ? t('settings.log.title.acp')
    : t('settings.log.title.console')
);
const switchLogFooterText = computed(() => {
  if (switchLogContext.value === 'acp') {
    return switchLogInProgress.value
      ? t('settings.log.footer.acp.pending')
      : t('settings.log.footer.acp.done');
  }
  return switchLogInProgress.value
    ? t('settings.log.footer.console.pending')
    : t('settings.log.footer.console.done');
});
const quickProfileConfirmOpen = ref(false);
const pendingQuickProfile = ref<string | null>(null);

function isAcpAuthTimeout(err: unknown) {
  const message = String(err).toLowerCase();
  return message.includes('timeout') || message.includes('timed out') || message.includes('超时');
}

function formatAcpAuthError(err: unknown) {
  if (isAcpAuthTimeout(err)) {
    return t('chat.authTimeout');
  }
  return t('chat.authFailed', { error: String(err) });
}

function isFileDragEvent(event: DragEvent) {
  const items = event.dataTransfer?.items;
  if (items) {
    for (const item of Array.from(items)) {
      if (item.kind === 'file') {
        return true;
      }
    }
  }
  const types = event.dataTransfer?.types;
  if (!types) {
    return false;
  }
  return Array.from(types).some((type) => type === 'Files' || type === 'public.file-url');
}

function showFileDropHint() {
  if (!isChatOpen.value) {
    isChatOpen.value = true;
  }
  isFileDragging.value = true;
}

function clearFileDropHint() {
  isFileDragging.value = false;
  if (dragIdleTimer !== null) {
    window.clearTimeout(dragIdleTimer);
    dragIdleTimer = null;
  }
}

function markFileDragActive() {
  lastFileDragAt = Date.now();
  if (dragIdleTimer !== null) {
    window.clearTimeout(dragIdleTimer);
  }
  dragIdleTimer = window.setTimeout(() => {
    if (!isFileDragging.value) {
      return;
    }
    if (Date.now() - lastFileDragAt >= DRAG_IDLE_TIMEOUT) {
      clearFileDropHint();
    }
  }, DRAG_IDLE_TIMEOUT);
}

function isDragLeavingWindow(event: DragEvent) {
  if (event.relatedTarget !== null) {
    return false;
  }
  const x = event.clientX;
  const y = event.clientY;
  return x <= 0 || y <= 0 || x >= window.innerWidth || y >= window.innerHeight;
}

function handleFileDragEnter(event: DragEvent) {
  if (!isFileDragEvent(event)) {
    return;
  }
  showFileDropHint();
  markFileDragActive();
}

function handleFileDragOver(event: DragEvent) {
  if (!isFileDragEvent(event)) {
    return;
  }
  showFileDropHint();
  markFileDragActive();
}

function handleFileDragLeave(event: DragEvent) {
  if (!isFileDragEvent(event)) {
    return;
  }
  if (isDragLeavingWindow(event)) {
    clearFileDropHint();
  }
}

function handleFileDragEnd() {
  clearFileDropHint();
}

function handleFileDrop(event: DragEvent) {
  if (!isFileDragEvent(event)) {
    return;
  }
  clearFileDropHint();
}

function resetSwitchLogState() {
  switchLogHasOutput.value = false;
  switchLogOffset.value = 0;
  switchLogStatusMessage.value = '';
  switchLogTerminal?.reset();
}

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

function resolveRgbaVar(name: string, fallback: string, alpha: number) {
  if (typeof window === 'undefined' || typeof document === 'undefined') {
    return `rgba(${fallback.replace(/\s+/g, ', ')}, ${alpha})`;
  }
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const value = raw || fallback;
  if (value.startsWith('rgb')) {
    const match = value.match(/rgba?\(([^)]+)\)/);
    if (match) {
      return `rgba(${match[1]}, ${alpha})`;
    }
    return `rgba(${fallback.replace(/\s+/g, ', ')}, ${alpha})`;
  }
  const normalized = value.replace(/\s+/g, ', ');
  return `rgba(${normalized}, ${alpha})`;
}

function resolveSwitchLogTheme(): ITheme {
  return {
    background: resolveRgbVar('--color-panel-muted', '30 41 59'),
    foreground: resolveRgbVar('--color-text', '226 232 240'),
    cursor: resolveRgbVar('--color-accent', '99 102 241'),
    selectionBackground: resolveRgbaVar('--color-accent', '99 102 241', 0.35),
  };
}

function resolveSwitchLogFontSize() {
  return SWITCH_LOG_BASE_FONT_SIZE * (settings.value.terminalScale || 1);
}

function ensureSwitchLogTerminal() {
  if (switchLogTerminal || !switchLogTerminalRef.value) {
    return;
  }
  switchLogTerminal = new Terminal({
    disableStdin: true,
    convertEol: true,
    fontSize: resolveSwitchLogFontSize(),
    fontFamily: 'Menlo, Monaco, \"Courier New\", monospace',
    theme: resolveSwitchLogTheme(),
    scrollback: 2000,
  });
  switchLogFitAddon = new FitAddon();
  switchLogTerminal.loadAddon(switchLogFitAddon);
  switchLogTerminal.open(switchLogTerminalRef.value);
  switchLogFitAddon.fit();
  switchLogResizeObserver = new ResizeObserver(() => {
    switchLogFitAddon?.fit();
  });
  switchLogResizeObserver.observe(switchLogTerminalRef.value);
}

function applySwitchLogTheme() {
  if (!switchLogTerminal) {
    return;
  }
  switchLogTerminal.options.theme = resolveSwitchLogTheme();
  if (switchLogTerminal.rows > 0) {
    switchLogTerminal.refresh(0, switchLogTerminal.rows - 1);
  }
}

function applySwitchLogScale() {
  if (!switchLogTerminal) {
    return;
  }
  switchLogTerminal.options.fontSize = resolveSwitchLogFontSize();
  switchLogFitAddon?.fit();
  if (switchLogTerminal.rows > 0) {
    switchLogTerminal.refresh(0, switchLogTerminal.rows - 1);
  }
}

async function pollSwitchLog() {
  try {
    const chunk =
      switchLogContext.value === 'acp'
        ? await readAppLog(switchLogOffset.value, 4096)
        : await readConsoleLog(switchLogOffset.value, 4096);
    if (chunk.content) {
      switchLogTerminal?.write(chunk.content);
      switchLogHasOutput.value = true;
    }
    switchLogOffset.value = chunk.nextOffset;
  } catch {
    // ignore polling errors
  }
}

async function startSwitchLogPolling(context: 'console' | 'acp') {
  switchLogContext.value = context;
  resetSwitchLogState();
  switchLogInProgress.value = true;
  switchLogOpen.value = true;
  await nextTick();
  ensureSwitchLogTerminal();
  applySwitchLogTheme();
  switchLogFitAddon?.fit();
  try {
    const chunk =
      switchLogContext.value === 'acp'
        ? await readAppLog(0, 0)
        : await readConsoleLog(0, 0);
    switchLogOffset.value = chunk.nextOffset;
  } catch {
    switchLogOffset.value = 0;
  }
  await pollSwitchLog();
  if (switchLogPollTimer !== null) {
    return;
  }
  switchLogPollTimer = window.setInterval(() => {
    void pollSwitchLog();
  }, 800);
}

function stopSwitchLogPolling() {
  if (switchLogPollTimer !== null) {
    window.clearInterval(switchLogPollTimer);
    switchLogPollTimer = null;
  }
  switchLogInProgress.value = false;
}

function disposeSwitchLogTerminal() {
  if (switchLogResizeObserver) {
    switchLogResizeObserver.disconnect();
    switchLogResizeObserver = null;
  }
  switchLogFitAddon = null;
  switchLogTerminal?.dispose();
  switchLogTerminal = null;
}

function closeSwitchLogModal() {
  if (switchLogInProgress.value) {
    return;
  }
  switchLogOpen.value = false;
}

async function refreshQuickProfiles() {
  if (!tauriAvailable) {
    return;
  }
  quickProfileLoading.value = true;
  try {
    const data = await listProfiles();
    quickProfiles.value = data.profiles;
    quickProfileCurrent.value = data.current;
  } catch (err) {
    const message = t('settings.profile.loadFailed', { error: String(err) });
    showNotification(message, undefined, undefined, 'error');
    reportUiError('load profiles failed', err);
  } finally {
    quickProfileLoading.value = false;
  }
}

async function handleQuickProfileSwitch(profileName: string) {
  if (!tauriAvailable) {
    return;
  }
  if (quickProfileSwitching.value) {
    return;
  }
  const current = quickProfileCurrent.value;
  if (!profileName || profileName === current) {
    return;
  }
  quickProfileSwitching.value = true;
  booting.value = true;
  hasConnected.value = false;
  connectionState.value = 'connecting';
  try {
    await startSwitchLogPolling('console');
    switchLogStatusMessage.value = t('settings.log.status.console.pending');
    await selectProfile(profileName);
    await restartConsole();
    switchLogStatusMessage.value = t('settings.log.status.console.done');
    showNotification(t('settings.apply.switchProfile', { name: profileName }));
    quickProfileCurrent.value = profileName;
    await startConsoleSession();
  } catch (err) {
    const message = t('settings.log.status.console.failed', { error: String(err) });
    switchLogStatusMessage.value = message;
    showNotification(message, undefined, undefined, 'error');
    quickProfileCurrent.value = current;
  } finally {
    stopSwitchLogPolling();
    quickProfileSwitching.value = false;
  }
}

function requestQuickProfileSwitch(profileName: string) {
  if (!profileName || profileName === quickProfileCurrent.value || quickProfileSwitching.value) {
    return;
  }
  pendingQuickProfile.value = profileName;
  quickProfileConfirmOpen.value = true;
}

function cancelQuickProfileSwitch() {
  quickProfileConfirmOpen.value = false;
  pendingQuickProfile.value = null;
}

function confirmQuickProfileSwitch() {
  const target = pendingQuickProfile.value;
  quickProfileConfirmOpen.value = false;
  pendingQuickProfile.value = null;
  if (target) {
    void handleQuickProfileSwitch(target);
  }
}

type ChatProviderInitOptions = {
  provider?: 'openai' | 'acp';
  config?: AppSettings['chat'];
};

// Initialize chat provider based on settings
function buildAcpArgs(config: AppSettings['chat']['acp']) {
  const args: string[] = [];
  const rawArgs = config.args.trim();
  if (rawArgs) {
    args.push(rawArgs);
  }
  if (config.approvalPolicy && config.approvalPolicy !== 'auto') {
    args.push(`--approval-policy ${config.approvalPolicy}`);
  }
  if (config.sandboxMode && config.sandboxMode !== 'auto') {
    args.push(`--sandbox-mode ${config.sandboxMode}`);
  }
  return args.join(' ').trim();
}

async function initChatProvider(options: ChatProviderInitOptions = {}) {
  const chatConfig = options.config ?? settings.value.chat;
  const provider = options.provider ?? chatConfig.provider;
  console.log('[initChatProvider] config:', provider);
  
  try {
    if (provider === 'openai') {
      await chatStore.initializeOpenai(chatConfig.openai, chatConfig.mcpConfigJson);
    } else {
      // ACP provider
      console.log('[initChatProvider] calling initializeAcp...');
      const acpArgs = buildAcpArgs(chatConfig.acp);
      await chatStore.initializeAcp('.', acpArgs, chatConfig.mcpConfigJson);
      console.log('[initChatProvider] initializeAcp done, providerInitialized:', providerInitialized.value);
      
      // Authentication is optional - don't fail if it's not available
      if ((chatStore.authMethods as AuthMethod[]).some((m) => m.id === 'openai-api-key')) {
        try {
          await chatStore.authenticateAcp('openai-api-key');
          console.log('[initChatProvider] authenticateAcp done');
        } catch (authErr) {
          console.warn('[initChatProvider] authenticateAcp failed (optional):', authErr);
          showNotification(formatAcpAuthError(authErr), undefined, undefined, 'error');
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

async function handleChatSend(options: SendMessageOptions) {
  console.log('[handleChatSend] providerInitialized:', providerInitialized.value, 'provider:', chatStore.provider);
  if (providerInitialized.value) {
    try {
      await chatStore.sendMessage(options);
    } catch (e) {
      showNotification(t('chat.error', { error: String(e) }), undefined, undefined, 'error');
    }
  } else {
    // Fallback to simulated response
    const fallbackContent = options.content ?? '';
    chatStore.addMessage({
      type: 'say',
      say: 'text',
      role: 'user',
      content: fallbackContent,
      images: options.images?.map((img) => img.previewUrl),
      files: options.files?.map((file) => file.name),
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

    const response = t('chat.fallbackResponse', { content: fallbackContent });
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

async function handleChatClear() {
  if (chatIsStreaming.value) {
    await cancelActiveChat();
  }
  chatStore.createSession();
}

function handleChatShowHistory() {
  isChatHistoryOpen.value = true;
  if (chatProvider.value === 'acp') {
    void chatStore.refreshAcpHistorySummaries();
  }
}

const openaiSessions = computed(() => chatStore.sessions.filter((s) => s.provider === 'openai'));
const acpHistoryMap = computed(() => {
  const map = new Map<string, AcpSessionSummary>();
  const sessionByAcpId = new Map(
    chatStore.sessions
      .filter((s) => s.provider === 'acp' && s.acpSessionId)
      .map((s) => [s.acpSessionId as string, s.id])
  );
  for (const summary of acpHistorySummaries.value) {
    const displayId = sessionByAcpId.get(summary.sessionId) ?? summary.sessionId;
    map.set(displayId, summary);
  }
  return map;
});
const acpHistorySessions = computed<ChatSession[]>(() => {
  const now = Date.now();
  return acpHistorySummaries.value.map((summary): ChatSession => {
    const existing = chatStore.sessions.find(
      (session) => session.provider === 'acp' && session.acpSessionId === summary.sessionId
    );
    const createdAt = summary.createdAt ?? summary.updatedAt ?? now;
    const updatedAt = summary.updatedAt ?? summary.createdAt ?? createdAt;
    return {
      id: existing?.id ?? summary.sessionId,
      provider: 'acp',
      title: summary.title || summary.sessionId,
      createdAt,
      updatedAt,
      messages: existing?.messages ?? [],
      messageCount: summary.messageCount,
      totalTokens: existing?.totalTokens ?? 0,
      status: existing?.status ?? 'idle',
      acpSessionId: summary.sessionId,
      plan: existing?.plan,
    };
  });
});
const historySessions = computed(() =>
  chatProvider.value === 'acp' ? acpHistorySessions.value : openaiSessions.value
);
const historyLoading = computed(() =>
  chatProvider.value === 'acp' ? acpHistoryLoading.value : false
);

function handleHistorySelect(sessionId: string) {
  if (chatProvider.value === 'acp') {
    const summary = acpHistoryMap.value.get(sessionId);
    const remoteId = summary?.sessionId ?? sessionId;
    chatStore.activateAcpHistorySession(remoteId, summary);
  } else {
    chatStore.setActiveSession(sessionId);
  }
  isChatHistoryOpen.value = false;
}

async function handleHistoryDelete(sessionId: string) {
  if (chatProvider.value === 'acp') {
    const summary = acpHistoryMap.value.get(sessionId);
    const remoteId = summary?.sessionId ?? sessionId;
    try {
      await chatStore.deleteAcpHistorySession(remoteId);
    } catch (err) {
      showNotification(t('chat.error', { error: String(err) }));
    }
    return;
  }
  chatStore.deleteSessionForProvider(sessionId, 'openai');
}

async function handleHistoryClear() {
  if (chatProvider.value === 'acp') {
    try {
      await chatStore.clearAcpHistorySessions();
    } catch (err) {
      showNotification(t('chat.error', { error: String(err) }));
    }
    return;
  }
  chatStore.clearSessionsForProvider('openai');
}

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
    showNotification(t('chat.providerSwitch.failed', { error: String(e) }), undefined, undefined, 'error');
  } finally {
    providerSwitching.value = false;
    providerSwitchConfirmOpen.value = false;
    pendingProvider.value = null;
  }
}

function showNotification(message: string, count?: number, target?: string, type?: 'success' | 'warning' | 'error' | 'info') {
  notification.value = { message, count, target, type };
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

async function startConsoleSession(): Promise<boolean> {
  const ok = await refreshTargets();
  void connectWebSocket();
  void logUiEvent(`origin=${window.location.origin} secure=${window.isSecureContext}`);
  return ok;
}

async function loadStartupProfiles() {
  if (!isTauri()) {
    return;
  }
  startupBusy.value = true;
  startupError.value = '';
  startupStatusMessage.value = t('console.startup.loading');
  try {
    const data = await listProfiles();
    startupProfiles.value = data.profiles;
    startupSelectedProfile.value = data.current || data.profiles[0]?.name || null;
    startupProfileOpen.value = true;
    connectionState.value = 'disconnected';
    startupStatusMessage.value = '';
  } catch (err) {
    const message = t('console.startup.loadFailed', { error: String(err) });
    startupError.value = message;
    showNotification(message, undefined, undefined, 'error');
    reportUiError('load profiles failed', err);
    startupProfileOpen.value = true;
  } finally {
    startupBusy.value = false;
  }
}

async function applyStartupProfile(): Promise<boolean> {
  if (startupBusy.value) {
    return false;
  }
  if (!startupSelectedProfile.value) {
  startupError.value = t('console.startup.selectProfile');
    return false;
  }
  startupBusy.value = true;
  startupError.value = '';
  booting.value = true;
  hasConnected.value = false;
  connectionState.value = 'connecting';
  startupStatusMessage.value = t('console.startup.applyingProfile', { name: startupSelectedProfile.value });
  try {
    await selectProfile(startupSelectedProfile.value);
    const status = await getProxyConfigStatus();
    if (!status.present) {
      const message = t('console.startup.configMissing', {
        path: status.path,
        example: status.example_path,
      });
      startupError.value = message;
      showNotification(message, undefined, undefined, 'error');
      connectionState.value = 'disconnected';
      booting.value = false;
      return false;
    }
    startupStatusMessage.value = t('console.startup.validating');
    const check = await validateStartupConfig();
    if (!check.ok) {
      const message = t('console.startup.validationFailed', { errors: check.errors.join('\n- ') });
      startupError.value = message;
      showNotification(t('console.startup.validationFailedToast'), undefined, undefined, 'error');
      reportUiError('startup config invalid', check.errors.join(' | '));
      connectionState.value = 'disconnected';
      booting.value = false;
      if (check.needs_setup) {
        openSettingsForConfig();
      }
      return false;
    }
    startupStatusMessage.value = t('console.startup.starting');
    await restartConsole();
    startupProfileOpen.value = false;
    startupStatusMessage.value = '';
    await startConsoleSession();
    if (typeof sessionStorage !== 'undefined') {
      sessionStorage.setItem(STARTUP_SESSION_KEY, '1');
    }
    return true;
  } catch (err) {
    const message = t('console.startup.startFailed', { error: String(err) });
    startupError.value = message;
    showNotification(message, undefined, undefined, 'error');
    reportUiError('startup profile failed', err);
    connectionState.value = 'disconnected';
    booting.value = false;
    return false;
  } finally {
    startupBusy.value = false;
  }
}

async function resumeConsoleSession() {
  const ok = await startConsoleSession();
  if (ok && connectionState.value === 'connecting') {
    connectionState.value = 'connected';
    hasConnected.value = true;
  }
  if (!ok) {
    await loadStartupProfiles();
  }
}

const { aiRiskMap, enqueueAiTask, processAiQueue, scheduleAiForSnapshot, updateAiRiskEntry } = useAiRiskQueue({
  settings,
  onError: reportUiError,
});

const autoApprovedLowRisk = new Set<string>();
let autoApproveLowRiskScheduled = false;

function isAutoApproveLowRiskEnabled() {
  return settings.value.ai.enabled && settings.value.ai.autoApproveLowRisk;
}

function autoApproveKey(targetName: string, requestId: string) {
  return `${targetName}:${requestId}`;
}

function resetAutoApproveLowRisk() {
  autoApprovedLowRisk.clear();
  autoApproveLowRiskScheduled = false;
}

function scheduleAutoApproveLowRisk() {
  if (!isAutoApproveLowRiskEnabled()) {
    return;
  }
  if (autoApproveLowRiskScheduled) {
    return;
  }
  autoApproveLowRiskScheduled = true;
  Promise.resolve().then(() => {
    autoApproveLowRiskScheduled = false;
    void processAutoApproveLowRisk();
  });
}

async function processAutoApproveLowRisk() {
  if (!isAutoApproveLowRiskEnabled()) {
    return;
  }
  const pendingKeys = new Set<string>();
  for (const [targetName, snapshot] of Object.entries(snapshots.value)) {
    const queue = snapshot?.queue ?? [];
    for (const request of queue) {
      const key = autoApproveKey(targetName, request.id);
      pendingKeys.add(key);
      if (autoApprovedLowRisk.has(key)) {
        continue;
      }
      const entry = aiRiskMap.value[key];
      if (!entry || entry.status !== 'done' || entry.risk !== 'low') {
        continue;
      }
      if (entry.autoApproved) {
        autoApprovedLowRisk.add(key);
        continue;
      }
      autoApprovedLowRisk.add(key);
      void approveCommand(targetName, request.id)
        .then(() => {
          updateAiRiskEntry(key, { autoApproved: true, autoApprovedAt: Date.now() });
        })
        .catch((err) => {
          autoApprovedLowRisk.delete(key);
          reportUiError('auto approve low risk failed', err);
        });
    }
  }
  for (const key of Array.from(autoApprovedLowRisk)) {
    if (!pendingKeys.has(key)) {
      autoApprovedLowRisk.delete(key);
    }
  }
}

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
  void chatStore.refreshOpenaiTools(list);
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
    showNotification(t('console.notifications.newRequest', { target: target.name }), target.pending_count, target.name);
  }
  if (shouldRefreshSnapshotFromTargetUpdate(target, previous)) {
    void refreshSnapshot(target.name);
  }
  lastPendingCounts.value[target.name] = target.pending_count;
  void chatStore.refreshOpenaiTools(targets.value);
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

async function refreshTargets(): Promise<boolean> {
  try {
    const list = await fetchTargets();
    updateTargets(list);
    return true;
  } catch (err) {
    connectionState.value = 'disconnected';
    reportUiError('fetch targets failed', err);
    return false;
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
    showNotification(t('console.notifications.approveFailed'), undefined, undefined, 'error');
    reportUiError('approve command failed', err);
  }
}

async function deny(id: string) {
  if (!selectedTargetName.value) return;
  try {
    await denyCommand(selectedTargetName.value, id);
  } catch (err) {
    showNotification(t('console.notifications.denyFailed'), undefined, undefined, 'error');
    reportUiError('deny command failed', err);
  }
}

async function cancel(id: string) {
  if (!selectedTargetName.value) return;
  try {
    await cancelCommand(selectedTargetName.value, id);
  } catch (err) {
    showNotification(t('console.notifications.cancelFailed'), undefined, undefined, 'error');
    reportUiError('cancel command failed', err);
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

function handleSettingsSave(value: AppSettings, close = true) {
  const previousSettings = settings.value;
  settings.value = value;
  if (close) {
    isSettingsOpen.value = false;
    clearSettingsPreview();
  }
  const providerChanged = previousSettings.chat.provider !== value.chat.provider;
  if (providerChanged && chatStore.provider !== value.chat.provider) {
    pendingProvider.value = value.chat.provider;
    providerSwitchConfirmOpen.value = true;
    void refreshChatProviderFromSettings(previousSettings, value, {
      allowProviderSwitch: false,
      activeProvider: chatStore.provider,
    });
  } else {
    void refreshChatProviderFromSettings(previousSettings, value);
  }
  void refreshQuickProfiles();
}

function handleSettingsPreview(
  key: 'language' | 'uiScale' | 'terminalScale',
  value: unknown
) {
  if (key === 'language') {
    previewLanguage.value =
      value === 'zh-CN' || value === 'en-US' ? (value as AppLanguage) : null;
    return;
  }
  if (key === 'uiScale') {
    previewUiScale.value = typeof value === 'number' ? value : null;
    return;
  }
  if (key === 'terminalScale') {
    previewTerminalScale.value = typeof value === 'number' ? value : null;
  }
}

function clearSettingsPreview() {
  previewLanguage.value = null;
  previewUiScale.value = null;
  previewTerminalScale.value = null;
}

function handleSettingsClose() {
  isSettingsOpen.value = false;
  clearSettingsPreview();
  void refreshQuickProfiles();
}

function hasOpenaiConfigChanged(previous: AppSettings, next: AppSettings) {
  const prevOpenai = previous.chat.openai;
  const nextOpenai = next.chat.openai;
  return (
    prevOpenai.baseUrl !== nextOpenai.baseUrl ||
    prevOpenai.apiKey !== nextOpenai.apiKey ||
    prevOpenai.model !== nextOpenai.model ||
    prevOpenai.chatPath !== nextOpenai.chatPath
  );
}

function hasMcpConfigChanged(previous: AppSettings, next: AppSettings) {
  return previous.chat.mcpConfigJson !== next.chat.mcpConfigJson;
}

function hasAcpConfigChanged(previous: AppSettings, next: AppSettings) {
  return (
    previous.chat.acp.args !== next.chat.acp.args ||
    previous.chat.acp.approvalPolicy !== next.chat.acp.approvalPolicy ||
    previous.chat.acp.sandboxMode !== next.chat.acp.sandboxMode ||
    previous.chat.mcpConfigJson !== next.chat.mcpConfigJson
  );
}

function hasAcpRestartSettingsChanged(previous: AppSettings, next: AppSettings) {
  return (
    previous.chat.acp.approvalPolicy !== next.chat.acp.approvalPolicy ||
    previous.chat.acp.sandboxMode !== next.chat.acp.sandboxMode ||
    previous.chat.mcpConfigJson !== next.chat.mcpConfigJson
  );
}

async function restartAcpSessionWithLog(createSession: boolean, config?: AppSettings['chat']) {
  if (acpRestarting.value) {
    return;
  }
  acpRestarting.value = true;
  chatStore.setConnected(false);
  try {
    await startSwitchLogPolling('acp');
    switchLogStatusMessage.value = t('settings.log.status.acp.pending');
    await initChatProvider({ provider: 'acp', config: config ?? settings.value.chat });
    if (createSession) {
      chatStore.createSession();
    }
    switchLogStatusMessage.value = t('settings.log.status.acp.done');
  } catch (err) {
    const message = t('settings.log.status.acp.failed', { error: String(err) });
    switchLogStatusMessage.value = message;
    showNotification(message, undefined, undefined, 'error');
  } finally {
    stopSwitchLogPolling();
    acpRestarting.value = false;
  }
}

type ChatProviderRefreshOptions = {
  allowProviderSwitch?: boolean;
  activeProvider?: 'openai' | 'acp';
};

async function refreshChatProviderFromSettings(
  previous: AppSettings,
  next: AppSettings,
  options: ChatProviderRefreshOptions = {}
) {
  const allowProviderSwitch = options.allowProviderSwitch ?? true;
  const providerChanged = previous.chat.provider !== next.chat.provider;
  const openaiChanged = hasOpenaiConfigChanged(previous, next);
  const mcpChanged = hasMcpConfigChanged(previous, next);
  const acpChanged = hasAcpConfigChanged(previous, next);
  const acpRestartChanged = hasAcpRestartSettingsChanged(previous, next);
  const activeProvider = options.activeProvider ?? chatStore.provider;
  const shouldSwitchProvider = allowProviderSwitch && providerChanged;
  const refreshProvider = shouldSwitchProvider ? next.chat.provider : activeProvider;
  const needsOpenaiRefresh = refreshProvider === 'openai' && (shouldSwitchProvider || openaiChanged);
  const needsAcpRefresh = refreshProvider === 'acp' && (shouldSwitchProvider || acpChanged);
  const mcpOnlyOpenaiChange =
    refreshProvider === 'openai' && mcpChanged && !needsOpenaiRefresh && !shouldSwitchProvider;

  if (mcpOnlyOpenaiChange) {
    chatStore.updateMcpConfig(next.chat.mcpConfigJson);
    await chatStore.refreshOpenaiTools(targets.value);
    return;
  }

  if (!needsOpenaiRefresh && !needsAcpRefresh) {
    return;
  }

  if (chatIsStreaming.value) {
    await cancelActiveChat();
  }

  if (shouldSwitchProvider) {
    await stopActiveProvider();
  } else if (refreshProvider === 'openai' && openaiChanged && chatStore.provider === 'openai') {
    await chatStore.stopOpenai();
  }

  if (refreshProvider === 'acp' && acpRestartChanged) {
    await restartAcpSessionWithLog(shouldSwitchProvider, next.chat);
    return;
  }

  try {
    await initChatProvider({ provider: refreshProvider, config: next.chat });
    if (shouldSwitchProvider) {
      chatStore.createSession();
    }
  } catch (err) {
    console.error('[Chat] settings refresh failed:', err);
    showNotification(t('chat.settingsRefreshFailed', { error: String(err) }), undefined, undefined, 'error');
  }
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
      showNotification(t('console.notifications.noPending'));
    }
  }
}

onMounted(async () => {
  void syncWindowMinSize();
  handleWindowResize();
  window.addEventListener('resize', handleWindowResize, { passive: true });
  window.addEventListener('keydown', handleGlobalKey);
  window.addEventListener('dragenter', handleFileDragEnter, true);
  window.addEventListener('dragover', handleFileDragOver, true);
  window.addEventListener('dragleave', handleFileDragLeave, true);
  window.addEventListener('drop', handleFileDrop, true);
  window.addEventListener('dragend', handleFileDragEnd, true);
  if (tauriAvailable) {
    if (typeof sessionStorage !== 'undefined' && sessionStorage.getItem(STARTUP_SESSION_KEY) === '1') {
      await resumeConsoleSession();
    } else {
      await loadStartupProfiles();
    }
    await refreshQuickProfiles();
    return;
  }
  try {
    const status = await getProxyConfigStatus();
    if (!status.present) {
      connectionState.value = 'disconnected';
      showNotification(t('console.startup.configMissingRestart', {
        path: status.path,
        example: status.example_path,
      }), undefined, undefined, 'error');
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
  window.removeEventListener('resize', handleWindowResize);
  if (windowResizeIdleTimer !== null) {
    window.clearTimeout(windowResizeIdleTimer);
    windowResizeIdleTimer = null;
  }
  window.removeEventListener('dragenter', handleFileDragEnter, true);
  window.removeEventListener('dragover', handleFileDragOver, true);
  window.removeEventListener('dragleave', handleFileDragLeave, true);
  window.removeEventListener('drop', handleFileDrop, true);
  window.removeEventListener('dragend', handleFileDragEnd, true);
  stopSwitchLogPolling();
  disposeSwitchLogTerminal();
  if (dragIdleTimer !== null) {
    window.clearTimeout(dragIdleTimer);
    dragIdleTimer = null;
  }
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
  () => effectiveLanguage.value,
  (value) => {
    locale.value = value;
    void setAppLanguage(value);
  },
  { immediate: true }
);

watch(
  () => settings.value.ai,
  (value, previous) => {
    if (value.enabled && (!previous?.enabled || previous?.apiKey !== value.apiKey)) {
      scheduleAiForAllTargets();
    }
    const autoApproveEnabled = value.enabled && value.autoApproveLowRisk;
    const previousAutoApproveEnabled = Boolean(previous?.enabled && previous?.autoApproveLowRisk);
    if (autoApproveEnabled && !previousAutoApproveEnabled) {
      scheduleAutoApproveLowRisk();
    }
    if (!autoApproveEnabled) {
      resetAutoApproveLowRisk();
    }
    processAiQueue();
  },
  { deep: true }
);

watch(aiRiskMap, () => {
  scheduleAutoApproveLowRisk();
});

watch(snapshots, () => {
  scheduleAutoApproveLowRisk();
});

watch(
  () => settings.value.theme,
  (mode) => {
    applyThemeMode(mode);
  },
  { immediate: true }
);

watch(
  () => effectiveUiScale.value,
  (value) => {
    void applyUiScale(value);
  },
  { immediate: true }
);

watch(
  () => resolvedTheme.value,
  () => {
    applySwitchLogTheme();
  }
);

watch(
  () => settings.value.terminalScale,
  () => {
    applySwitchLogScale();
  }
);
</script>

<template>
  <NotificationBridge :payload="notification" :token="notificationToken" @jump-pending="handleNotificationJump" />
  <n-modal v-model:show="startupProfileOpen" :mask-closable="false" :close-on-esc="false">
    <n-card size="small" class="w-[26rem]" :bordered="true">
      <template #header>{{ $t('console.startup.title') }}</template>
      <div class="space-y-3">
        <div class="text-xs text-foreground-muted">
          {{ $t('console.startup.subtitle') }}
        </div>
        <NSelect
          :value="startupSelectedProfile"
          :options="startupProfileOptions"
          size="small"
          :disabled="startupBusy || startupProfileOptions.length === 0"
          :placeholder="$t('console.startup.placeholder')"
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
          <n-button size="small" :disabled="startupBusy" @click="isSettingsOpen = true">{{ $t('settings.title') }}</n-button>
          <n-button
            size="small"
            type="primary"
            :disabled="startupBusy || !startupSelectedProfile"
            @click="applyStartupProfile"
          >
            {{ startupBusy ? $t('console.startup.startingButton') : $t('console.startup.startButton') }}
          </n-button>
        </div>
      </div>
    </n-card>
  </n-modal>
  <div
    class="flex h-screen w-screen bg-surface text-foreground overflow-hidden"
    :style="{ paddingTop: dragRegionHeight }"
  >
    <div
      class="fixed top-0 left-0 right-0 z-[4000] pointer-events-auto"
      :style="{ height: dragRegionHeight }"
      data-tauri-drag-region
    ></div>
    <ConsoleLeftPane
      ref="leftPaneRef"
      :targets="targets"
      :sidebar-width="layoutSizing.sidebarWidth"
      :selected-target-name="selectedTargetName"
      :pending-total="pendingTotal"
      :connection-state="connectionState"
      :profiles="quickProfiles"
      :active-profile="quickProfileCurrent"
      :profiles-enabled="tauriAvailable"
      :profile-loading="quickProfileLoading"
      :profile-switching="quickProfileSwitching"
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
      :terminal-scale="effectiveTerminalScale"
      :resolved-theme="resolvedTheme"
      @select-target="selectedTargetName = $event"
      @open-settings="isSettingsOpen = true"
      @switch-profile="requestQuickProfileSwitch"
      @toggle-chat="isChatOpen = !isChatOpen"
      @approve="approve"
      @deny="deny"
      @cancel="cancel"
      @refresh-risk="refreshAiRisk"
      @open-terminal="openSelectedTerminal"
      @close-terminal="closeSelectedTerminal"
      @terminal-add="handleAddTerminalTab"
      @terminal-close="handleCloseTerminalTab"
      @terminal-activate="handleActivateTerminalTab"
    />

    <ConsoleChatPane
      :is-chat-open="isChatOpen"
      :show-drop-hint="showChatDropHint"
      :chat-min-width="CHAT_MIN_WIDTH"
      :chat-max-width="chatMaxWidth"
      :disable-transition="isWindowResizing"
      :messages="chatMessages"
      :plan-entries="chatPlanEntries"
      :is-streaming="chatIsStreaming"
      :is-connected="chatIsConnected"
      :input-locked="chatInputLocked"
      :provider="chatProvider"
      :send-on-enter="settings.chat.sendOnEnter"
      :supports-images="providerSupportsImages"
      :targets="targets"
      :is-history-open="isChatHistoryOpen"
      :history-sessions="historySessions"
      :history-loading="historyLoading"
      :active-session-id="chatStore.activeSessionId"
      @send="handleChatSend"
      @cancel="handleChatCancel"
      @width-change="handleChatWidthChange"
      @show-history="handleChatShowHistory"
      @clear="handleChatClear"
      @change-provider="handleChangeProvider"
      @close-history="isChatHistoryOpen = false"
      @select-session="handleHistorySelect"
      @delete-session="handleHistoryDelete"
      @clear-sessions="handleHistoryClear"
    />

    <SettingsModal
      :is-open="isSettingsOpen"
      :settings="settings"
      :resolved-theme="resolvedTheme"
      :focus-config-token="focusConfigToken"
      @close="handleSettingsClose"
      @save="handleSettingsSave"
      @preview="handleSettingsPreview"
    />

  <n-modal v-model:show="providerSwitchConfirmOpen" :mask-closable="false" :close-on-esc="true">
    <n-card size="small" class="w-[24rem]" :bordered="true">
      <template #header>{{ $t('chat.providerSwitch.title') }}</template>
      <div class="space-y-2 text-sm text-foreground-muted">
        <div>{{ $t('chat.providerSwitch.hint', { provider: pendingProviderLabel }) }}</div>
        <div>{{ $t('chat.providerSwitch.subHint') }}</div>
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button :disabled="providerSwitching" @click="cancelProviderSwitch">{{ $t('common.cancel') }}</n-button>
          <n-button type="primary" :disabled="providerSwitching" @click="confirmProviderSwitch">
            {{ $t('chat.providerSwitch.confirm') }}
          </n-button>
        </div>
      </template>
    </n-card>
  </n-modal>

  <n-modal v-model:show="quickProfileConfirmOpen" :mask-closable="false" :close-on-esc="true">
    <n-card size="small" class="w-[24rem]" :bordered="true">
      <template #header>{{ $t('settings.profile.quickSwitchTitle') }}</template>
      <div class="text-sm text-foreground-muted">
        {{ $t('settings.profile.quickSwitchHint', { name: pendingQuickProfile }) }}
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button @click="cancelQuickProfileSwitch">{{ $t('common.cancel') }}</n-button>
          <n-button type="primary" @click="confirmQuickProfileSwitch">
            {{ $t('settings.profile.quickSwitchConfirm') }}
          </n-button>
        </div>
      </template>
    </n-card>
  </n-modal>

  <n-modal v-model:show="switchLogOpen" :mask-closable="false" :close-on-esc="false">
    <n-card size="small" class="w-[36rem]" :bordered="true">
      <template #header>{{ switchLogTitle }}</template>
      <div class="text-sm text-foreground-muted">
        {{ switchLogStatusMessage || $t('settings.log.preparing') }}
      </div>
      <div class="relative mt-3 h-64 overflow-hidden rounded border border-border bg-panel-muted">
        <div ref="switchLogTerminalRef" class="h-full w-full" />
        <div
          v-if="!switchLogHasOutput"
          class="absolute inset-0 flex items-center justify-center text-xs text-foreground-muted pointer-events-none"
        >
          {{ $t('settings.log.empty') }}
        </div>
      </div>
      <template #footer>
        <div class="flex items-center justify-between gap-3">
          <div class="flex items-center gap-2 text-xs text-foreground-muted">
            <n-spin v-if="switchLogInProgress" size="small" />
            <span>{{ switchLogFooterText }}</span>
          </div>
          <n-button type="primary" :disabled="switchLogInProgress" @click="closeSwitchLogModal">
            {{ $t('common.done') }}
          </n-button>
        </div>
      </template>
    </n-card>
  </n-modal>

  </div>
</template>
