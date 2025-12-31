<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import {
  approveCommand,
  aiRiskAssess,
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
import { matchesShortcut } from './shortcuts';
import Sidebar from './components/Sidebar.vue';
import TerminalPanel from './components/TerminalPanel.vue';
import TargetView from './components/TargetView.vue';
import SettingsModal from './components/SettingsModal.vue';
import NotificationBridge from './components/NotificationBridge.vue';
import { loadSettings, saveSettings } from './settings';
import type {
  AiRiskEntry,
  AiRiskApiResponse,
  AppSettings,
  ConsoleEvent,
  RequestSnapshot,
  ServiceSnapshot,
  TargetInfo,
  ThemeMode,
} from './types';
import { startWindowDrag } from './tauriWindow';

const targets = ref<TargetInfo[]>([]);
const snapshots = ref<Record<string, ServiceSnapshot>>({});
const selectedTargetName = ref<string | null>(null);
const settings = ref(loadSettings());
const isSettingsOpen = ref(false);
const notification = ref<{ message: string; count?: number } | null>(null);
const notificationToken = ref(0);
const connectionState = ref<'connected' | 'connecting' | 'disconnected'>('connecting');
const snapshotLoading = ref<Record<string, boolean>>({});
const pendingJumpToken = ref(0);
type TerminalTab = {
  id: string;
  label: string;
  createdAt: number;
};

type TerminalTargetState = {
  open: boolean;
  tabs: TerminalTab[];
  activeId: string | null;
  nextIndex: number;
};

const terminalState = ref<Record<string, TerminalTargetState>>({});
const resolvedTheme = ref<'dark' | 'light'>('dark');
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
let stopSystemThemeListener: (() => void) | null = null;
const AI_RISK_CACHE_KEY = 'octovalve.console.ai_risk_cache';
const AI_RISK_CACHE_LIMIT = 2000;

type AiTask = {
  target: string;
  request: RequestSnapshot;
};

const aiRiskMap = ref<Record<string, AiRiskEntry>>(loadAiRiskCache());
const aiQueue = ref<AiTask[]>([]);
const aiQueuedKeys = new Set<string>();
const aiInFlightKeys = new Set<string>();
const aiRunning = ref(0);
let aiPumpScheduled = false;
let aiPersistTimer: number | null = null;

const pendingTotal = computed(() => targets.value.reduce((sum, target) => sum + target.pending_count, 0));
const selectedTarget = computed(() => targets.value.find((target) => target.name === selectedTargetName.value) ?? null);
const selectedTerminal = computed<TerminalTargetState>(() => {
  if (!selectedTargetName.value) {
    return { open: false, tabs: [], activeId: null, nextIndex: 1 };
  }
  return (
    terminalState.value[selectedTargetName.value] ?? {
      open: false,
      tabs: [],
      activeId: null,
      nextIndex: 1,
    }
  );
});
const selectedTerminalOpen = computed(
  () => selectedTerminal.value.open && selectedTerminal.value.tabs.length > 0
);
const selectedSnapshot = computed(() => {
  if (!selectedTargetName.value) {
    return null;
  }
  return snapshots.value[selectedTargetName.value] ?? null;
});

const terminalEntries = computed(() =>
  targets.value
    .map((target) => ({ target, state: terminalState.value[target.name] }))
    .filter((entry) => entry.state && entry.state.tabs.length > 0)
    .map((entry) => ({ target: entry.target, state: entry.state! }))
);

const selectedTerminalEntry = computed(() => {
  if (!selectedTargetName.value) {
    return null;
  }
  return terminalEntries.value.find((item) => item.target.name === selectedTargetName.value) ?? null;
});
const activeTerminalTabId = computed<string | number | undefined>(() => {
  return selectedTerminalEntry.value?.state.activeId ?? undefined;
});

function createTabId() {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID();
  }
  return `term-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function createTerminalTab(index: number): TerminalTab {
  return {
    id: createTabId(),
    label: `Session ${index}`,
    createdAt: Date.now(),
  };
}

function setTerminalState(name: string, state: TerminalTargetState) {
  terminalState.value = {
    ...terminalState.value,
    [name]: state,
  };
}

function openTerminalForTarget(name: string) {
  const current = terminalState.value[name];
  if (!current) {
    const tab = createTerminalTab(1);
    setTerminalState(name, { open: true, tabs: [tab], activeId: tab.id, nextIndex: 2 });
    return;
  }
  if (current.tabs.length === 0) {
    const tab = createTerminalTab(current.nextIndex || 1);
    setTerminalState(name, {
      ...current,
      open: true,
      tabs: [tab],
      activeId: tab.id,
      nextIndex: (current.nextIndex || 1) + 1,
    });
    return;
  }
  if (!current.open) {
    setTerminalState(name, { ...current, open: true });
  }
}

function hideTerminalForTarget(name: string) {
  const current = terminalState.value[name];
  if (!current) {
    return;
  }
  if (current.open) {
    setTerminalState(name, { ...current, open: false });
  }
}

function addTerminalTab(name: string) {
  const current = terminalState.value[name] ?? {
    open: true,
    tabs: [],
    activeId: null,
    nextIndex: 1,
  };
  const tab = createTerminalTab(current.nextIndex || 1);
  setTerminalState(name, {
    ...current,
    open: true,
    tabs: [...current.tabs, tab],
    activeId: tab.id,
    nextIndex: (current.nextIndex || 1) + 1,
  });
}

function handleAddTerminalTab() {
  const entry = selectedTerminalEntry.value;
  if (!entry) {
    return;
  }
  addTerminalTab(entry.target.name);
}

function handleCloseTerminalTab(name: string | number) {
  const entry = selectedTerminalEntry.value;
  if (!entry) {
    return;
  }
  closeTerminalTab(entry.target.name, String(name));
}

function handleActivateTerminalTab(value: string | number) {
  const entry = selectedTerminalEntry.value;
  if (!entry) {
    return;
  }
  activateTerminalTab(entry.target.name, String(value));
}

function activateTerminalTab(name: string, tabId: string) {
  const current = terminalState.value[name];
  if (!current || current.activeId === tabId) {
    return;
  }
  setTerminalState(name, { ...current, activeId: tabId, open: true });
}

function closeTerminalTab(name: string, tabId: string) {
  const current = terminalState.value[name];
  if (!current) {
    return;
  }
  const index = current.tabs.findIndex((tab) => tab.id === tabId);
  if (index === -1) {
    return;
  }
  const nextTabs = current.tabs.filter((tab) => tab.id !== tabId);
  if (nextTabs.length === 0) {
    setTerminalState(name, {
      ...current,
      open: false,
      tabs: [],
      activeId: null,
    });
    return;
  }
  const nextActiveId =
    current.activeId === tabId ? nextTabs[Math.min(index, nextTabs.length - 1)].id : current.activeId;
  setTerminalState(name, {
    ...current,
    tabs: nextTabs,
    activeId: nextActiveId ?? nextTabs[0].id,
  });
}

function updateResolvedTheme(resolved: 'dark' | 'light', mode: ThemeMode) {
  resolvedTheme.value = resolved;
  document.documentElement.dataset.theme = resolved;
  document.documentElement.dataset.themeMode = mode;
}

function applyThemeMode(mode: ThemeMode) {
  if (typeof window === 'undefined' || typeof document === 'undefined') {
    return;
  }
  if (stopSystemThemeListener) {
    stopSystemThemeListener();
    stopSystemThemeListener = null;
  }
  if (mode === 'system') {
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const update = () => updateResolvedTheme(media.matches ? 'dark' : 'light', mode);
    update();
    const handler = () => update();
    if (typeof media.addEventListener === 'function') {
      media.addEventListener('change', handler);
      stopSystemThemeListener = () => media.removeEventListener('change', handler);
    } else {
      media.addListener(handler);
      stopSystemThemeListener = () => media.removeListener(handler);
    }
    return;
  }
  updateResolvedTheme(mode, mode);
}

function handleOpenTerminal() {
  if (!selectedTargetName.value) {
    return;
  }
  openTerminalForTarget(selectedTargetName.value);
}

function handleCloseTerminal() {
  if (!selectedTargetName.value) {
    return;
  }
  hideTerminalForTarget(selectedTargetName.value);
}

function showNotification(message: string, count?: number) {
  notification.value = { message, count };
  notificationToken.value += 1;
}

function reportUiError(context: string, err?: unknown) {
  const detail = err ? `: ${String(err)}` : '';
  void logUiEvent(`${context}${detail}`);
}

function loadAiRiskCache(): Record<string, AiRiskEntry> {
  if (typeof localStorage === 'undefined') {
    return {};
  }
  try {
    const raw = localStorage.getItem(AI_RISK_CACHE_KEY);
    if (!raw) {
      return {};
    }
    const parsed = JSON.parse(raw) as Record<string, AiRiskEntry>;
    if (!parsed || typeof parsed !== 'object') {
      return {};
    }
    const normalized: Record<string, AiRiskEntry> = {};
    for (const [key, value] of Object.entries(parsed)) {
      if (!value || typeof value !== 'object') {
        continue;
      }
      if (typeof value.updatedAt !== 'number') {
        continue;
      }
      normalized[key] = value;
    }
    return normalized;
  } catch (err) {
    reportUiError('ai risk cache load failed', err);
    return {};
  }
}

function scheduleAiRiskPersist() {
  if (typeof localStorage === 'undefined') {
    return;
  }
  if (aiPersistTimer !== null) {
    return;
  }
  aiPersistTimer = window.setTimeout(() => {
    aiPersistTimer = null;
    persistAiRiskCache();
  }, 500);
}

function persistAiRiskCache() {
  if (typeof localStorage === 'undefined') {
    return;
  }
  const entries = Object.entries(aiRiskMap.value);
  if (entries.length > AI_RISK_CACHE_LIMIT) {
    entries.sort(([, a], [, b]) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));
    const trimmed = Object.fromEntries(entries.slice(0, AI_RISK_CACHE_LIMIT));
    aiRiskMap.value = trimmed;
  }
  localStorage.setItem(AI_RISK_CACHE_KEY, JSON.stringify(aiRiskMap.value));
}

function setAiRisk(key: string, entry: AiRiskEntry) {
  aiRiskMap.value = {
    ...aiRiskMap.value,
    [key]: entry,
  };
  scheduleAiRiskPersist();
}

function buildAiKey(targetName: string, requestId: string) {
  return `${targetName}:${requestId}`;
}

function formatAiPipeline(request: RequestSnapshot) {
  return request.pipeline.map((stage) => stage.argv.join(' ')).join(' | ');
}

function buildAiPrompt(template: string, targetName: string, request: RequestSnapshot) {
  const replacements: Record<string, string> = {
    target: targetName,
    client: request.client,
    peer: request.peer,
    intent: request.intent,
    mode: request.mode,
    raw_command: request.raw_command,
    pipeline: formatAiPipeline(request),
    cwd: request.cwd ?? '-',
    timeout_ms: request.timeout_ms?.toString() ?? '-',
    max_output_bytes: request.max_output_bytes?.toString() ?? '-',
  };
  return template.replace(/\{\{(\w+)\}\}/g, (match, key) => {
    if (key in replacements) {
      return replacements[key];
    }
    return match;
  });
}

function enqueueAiTask(targetName: string, request: RequestSnapshot) {
  if (!settings.value.ai.enabled) {
    return;
  }
  const key = buildAiKey(targetName, request.id);
  const existing = aiRiskMap.value[key];
  if (aiQueuedKeys.has(key) || aiInFlightKeys.has(key)) {
    return;
  }
  if (!settings.value.ai.apiKey.trim()) {
    if (!existing || existing.status !== 'done') {
      setAiRisk(key, { status: 'error', error: '未配置 API Key', updatedAt: Date.now() });
    }
    return;
  }
  aiQueuedKeys.add(key);
  aiQueue.value = [...aiQueue.value, { target: targetName, request }];
  setAiRisk(key, { status: 'pending', updatedAt: Date.now() });
  scheduleAiQueue();
}

function scheduleAiQueue() {
  if (aiPumpScheduled) {
    return;
  }
  aiPumpScheduled = true;
  Promise.resolve().then(() => {
    aiPumpScheduled = false;
    processAiQueue();
  });
}

function resetAiQueue() {
  aiQueue.value = [];
  aiQueuedKeys.clear();
}

function processAiQueue() {
  if (!settings.value.ai.enabled) {
    resetAiQueue();
    return;
  }
  const maxConcurrency = Math.max(1, settings.value.ai.maxConcurrency);
  while (aiRunning.value < maxConcurrency && aiQueue.value.length > 0) {
    const task = aiQueue.value.shift();
    if (!task) {
      break;
    }
    const key = buildAiKey(task.target, task.request.id);
    aiQueuedKeys.delete(key);
    aiInFlightKeys.add(key);
    aiRunning.value += 1;
    void runAiTask(task)
      .catch((err) => {
        reportUiError('ai task failed', err);
        setAiRisk(key, { status: 'error', error: `AI 执行异常：${String(err)}`, updatedAt: Date.now() });
      })
      .finally(() => {
        aiRunning.value -= 1;
        aiInFlightKeys.delete(key);
        scheduleAiQueue();
      });
  }
}

async function runAiTask(task: AiTask) {
  const key = buildAiKey(task.target, task.request.id);
  const now = Date.now();
  if (!settings.value.ai.apiKey.trim()) {
    setAiRisk(key, { status: 'error', error: '未配置 API Key', updatedAt: now });
    return;
  }
  try {
    const prompt = buildAiPrompt(settings.value.ai.prompt, task.target, task.request);
    const response = await aiRiskAssess({
      base_url: settings.value.ai.baseUrl,
      chat_path: settings.value.ai.chatPath,
      model: settings.value.ai.model,
      api_key: settings.value.ai.apiKey,
      prompt,
      timeout_ms: settings.value.ai.timeoutMs,
    });
    applyAiResult(key, response);
  } catch (err) {
    setAiRisk(key, { status: 'error', error: String(err), updatedAt: now });
    reportUiError('ai risk assess failed', err);
  }
}

function applyAiResult(key: string, response: AiRiskApiResponse) {
  setAiRisk(key, {
    status: 'done',
    risk: response.risk,
    reason: response.reason,
    keyPoints: response.key_points ?? [],
    updatedAt: Date.now(),
  });
}

function scheduleAiForSnapshot(targetName: string, snapshot: ServiceSnapshot) {
  if (!settings.value.ai.enabled) {
    return;
  }
  const pending = snapshot.queue ?? [];
  if (pending.length === 0) {
    return;
  }
  pending.forEach((item) => {
    const key = buildAiKey(targetName, item.id);
    const existing = aiRiskMap.value[key];
    if (
      !existing ||
      (existing.status === 'error' && existing.error?.includes('API Key') && settings.value.ai.apiKey.trim())
    ) {
      enqueueAiTask(targetName, item);
    }
  });
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
  if (stopSystemThemeListener) {
    stopSystemThemeListener();
  }
  if (aiPersistTimer !== null) {
    window.clearTimeout(aiPersistTimer);
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
            @open-terminal="handleOpenTerminal"
            @close-terminal="handleCloseTerminal"
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
      </div>
    </n-notification-provider>
  </n-config-provider>
</template>
