<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch, type CSSProperties } from 'vue';
import { Terminal, type ITheme } from 'xterm';
import { FitAddon } from 'xterm-addon-fit';
import 'xterm/css/xterm.css';
import {
  NAlert,
  NButton,
  NCard,
  NInput,
  NInputNumber,
  NModal,
  NSelect,
  NSpin,
  NSwitch,
  NTabPane,
  NTabs,
  type SelectOption,
} from 'naive-ui';
import {
  createProfile,
  deleteProfile,
  listProfiles,
  readConsoleLog,
  readProfileBrokerConfig,
  readProfileProxyConfig,
  reloadRemoteBrokers,
  restartConsole,
  selectProfile,
  writeProfileBrokerConfig,
  writeProfileProxyConfig,
} from '../api';
import { eventToShortcut, formatShortcut, normalizeShortcut } from '../shortcuts';
import { DEFAULT_SETTINGS } from '../settings';
import type { AppSettings, ConfigFilePayload, ProfileSummary } from '../types';
import MonacoEditor from './MonacoEditor.vue';

const props = defineProps<{
  isOpen: boolean;
  settings: AppSettings;
  resolvedTheme: 'dark' | 'light';
}>();

const emit = defineEmits<{
  (e: 'close'): void;
  (e: 'save', settings: AppSettings): void;
}>();

function cloneSettings(source: AppSettings): AppSettings {
  return {
    notificationsEnabled: source.notificationsEnabled,
    theme: source.theme,
    ai: { ...source.ai },
    shortcuts: { ...source.shortcuts },
  };
}

const localSettings = ref<AppSettings>(cloneSettings(props.settings));
const activeTab = ref<'general' | 'shortcuts' | 'ai' | 'config'>('general');
const themeOptions: SelectOption[] = [
  { value: 'system', label: '系统' },
  { value: 'dark', label: '深色' },
  { value: 'light', label: '浅色' },
];
const aiProviderOptions: SelectOption[] = [{ value: 'openai', label: 'OpenAI 兼容' }];
const shortcutFields = [
  { key: 'prevTarget', label: '上一个目标' },
  { key: 'nextTarget', label: '下一个目标' },
  { key: 'jumpNextPending', label: '跳转到下一个 Pending' },
  { key: 'approve', label: '批准' },
  { key: 'deny', label: '拒绝' },
  { key: 'toggleList', label: '切换 Pending/History' },
  { key: 'fullScreen', label: '全屏输出' },
] as const;
type ShortcutField = (typeof shortcutFields)[number]['key'];
const activeShortcut = ref<ShortcutField | null>(null);
const configLoading = ref(false);
const configBusy = ref(false);
const configError = ref<string | null>(null);
const configMessage = ref<string | null>(null);
const configMessageType = ref<'success' | 'error' | 'warning' | 'info'>('success');
const profiles = ref<ProfileSummary[]>([]);
const activeProfile = ref<string | null>(null);
const selectedProfile = ref<string | null>(null);
const pendingProfile = ref<string | null>(null);
const switchProfileOpen = ref(false);
const createProfileOpen = ref(false);
const createProfileName = ref('');
const deleteProfileOpen = ref(false);
const deleteProfileName = ref<string | null>(null);
const refreshConfirmOpen = ref(false);
const confirmApplyOpen = ref(false);
const proxyConfig = ref<ConfigFilePayload | null>(null);
const brokerConfig = ref<ConfigFilePayload | null>(null);
const proxyConfigText = ref('');
const brokerConfigText = ref('');
const proxyOriginal = ref('');
const brokerOriginal = ref('');
const proxyApplied = ref<string | null>(null);
const brokerApplied = ref<string | null>(null);
const configLoaded = ref(false);
const logModalOpen = ref(false);
const logInProgress = ref(false);
const logContext = ref<'remote-broker' | 'console'>('remote-broker');
const logStatusMessage = ref('');
const logHasOutput = ref(false);
const logOffset = ref(0);
const logTerminalRef = ref<HTMLDivElement | null>(null);
let logPollTimer: number | null = null;
let logTerminal: Terminal | null = null;
let logFitAddon: FitAddon | null = null;
let logResizeObserver: ResizeObserver | null = null;
let configMessageTimer: number | null = null;
const cardShellRef = ref<HTMLDivElement | null>(null);
const cardInnerRef = ref<HTMLDivElement | null>(null);
const cardHeight = ref<number | null>(null);
const cardShellReady = ref(false);
let cardResizeObserver: ResizeObserver | null = null;

watch(
  () => props.settings,
  (value) => {
    localSettings.value = cloneSettings(value);
  },
  { deep: true }
);

const hasOpen = computed(() => props.isOpen);
const isConfigTab = computed(() => activeTab.value === 'config');
const proxyDirty = computed(() => proxyConfigText.value !== proxyOriginal.value);
const brokerDirty = computed(() => brokerConfigText.value !== brokerOriginal.value);
const profileOptions = computed<SelectOption[]>(() =>
  profiles.value.map((profile) => ({ value: profile.name, label: profile.name }))
);
const deletableProfileOptions = computed<SelectOption[]>(() =>
  profiles.value
    .filter((profile) => profile.name !== activeProfile.value)
    .map((profile) => ({ value: profile.name, label: profile.name }))
);
const canDeleteProfile = computed(() => deletableProfileOptions.value.length > 0);
const createProfileValid = computed(() => /^[A-Za-z0-9_-]{1,48}$/.test(createProfileName.value.trim()));
const logTitle = computed(() => (logContext.value === 'console' ? 'Console 重启日志' : '远端重启日志'));
const logFooterText = computed(() => {
  if (logContext.value === 'console') {
    return logInProgress.value ? '正在重启 console…' : 'console 重启流程已结束';
  }
  return logInProgress.value ? '正在重启远端 broker…' : '远端重启流程已结束';
});
const isAiTab = computed(() => activeTab.value === 'ai');
const cardMaxWidth = computed(() => (isConfigTab.value ? '80rem' : isAiTab.value ? '64rem' : '32rem'));
const cardShellStyle = computed<CSSProperties>(() => ({
  width: '100%',
  maxWidth: cardMaxWidth.value,
  height: cardHeight.value ? `${cardHeight.value}px` : 'auto',
}));
const cardStyle = computed(() => ({
  width: '100%',
  height: isConfigTab.value ? '80vh' : 'auto',
}));
const cardContentStyle = computed<CSSProperties>(() => {
  if (!isConfigTab.value) {
    return {};
  }
  return {
    display: 'flex',
    flexDirection: 'column',
    height: '100%',
  };
});

async function syncCardHeight() {
  if (!hasOpen.value) {
    return;
  }
  await nextTick();
  await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  const inner = cardInnerRef.value;
  if (!inner) {
    return;
  }
  const measuredHeight = Math.max(inner.scrollHeight, inner.offsetHeight);
  const nextHeight = Math.ceil(measuredHeight);
  if (!nextHeight) {
    return;
  }
  setCardHeight(nextHeight);
  startCardObserver();
}

function setCardHeight(nextHeight: number) {
  if (!nextHeight) {
    return;
  }
  cardHeight.value = Math.ceil(nextHeight);
  if (!cardShellReady.value) {
    cardShellReady.value = true;
  }
}

function startCardObserver() {
  if (typeof ResizeObserver === 'undefined') {
    return;
  }
  const inner = cardInnerRef.value;
  if (!inner) {
    return;
  }
  if (!cardResizeObserver) {
    cardResizeObserver = new ResizeObserver((entries) => {
      const entry = entries[entries.length - 1];
      if (!entry) {
        return;
      }
      setCardHeight(entry.contentRect.height);
    });
  }
  cardResizeObserver.observe(inner);
}

function stopCardObserver() {
  if (!cardResizeObserver) {
    return;
  }
  cardResizeObserver.disconnect();
  cardResizeObserver = null;
}

function save() {
  emit('save', cloneSettings(localSettings.value));
}

function showConfigMessage(message: string, type: 'success' | 'error' | 'warning' | 'info' = 'success') {
  configMessage.value = message;
  configMessageType.value = type;
  if (configMessageTimer !== null) {
    window.clearTimeout(configMessageTimer);
  }
  configMessageTimer = window.setTimeout(() => {
    configMessage.value = null;
    configMessageTimer = null;
  }, 4000);
}

function resetLogState() {
  logHasOutput.value = false;
  logOffset.value = 0;
  logStatusMessage.value = '';
  logTerminal?.reset();
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

function resolveLogTheme(): ITheme {
  return {
    background: resolveRgbVar('--color-panel-muted', '30 41 59'),
    foreground: resolveRgbVar('--color-text', '226 232 240'),
    cursor: resolveRgbVar('--color-accent', '99 102 241'),
    selectionBackground: 'rgba(99, 102, 241, 0.35)',
  };
}

function ensureLogTerminal() {
  if (logTerminal || !logTerminalRef.value) {
    return;
  }
  logTerminal = new Terminal({
    disableStdin: true,
    convertEol: true,
    fontSize: 12,
    fontFamily: 'Menlo, Monaco, "Courier New", monospace',
    theme: resolveLogTheme(),
    scrollback: 2000,
  });
  logFitAddon = new FitAddon();
  logTerminal.loadAddon(logFitAddon);
  logTerminal.open(logTerminalRef.value);
  logFitAddon.fit();
  logResizeObserver = new ResizeObserver(() => {
    logFitAddon?.fit();
  });
  logResizeObserver.observe(logTerminalRef.value);
}

function applyLogTheme() {
  if (!logTerminal) {
    return;
  }
  logTerminal.options.theme = resolveLogTheme();
  if (logTerminal.rows > 0) {
    logTerminal.refresh(0, logTerminal.rows - 1);
  }
}

function appendLogChunk(chunk: string) {
  if (!chunk) {
    return;
  }
  ensureLogTerminal();
  logTerminal?.write(chunk);
  logHasOutput.value = true;
}

async function pollConsoleLog() {
  try {
    const chunk = await readConsoleLog(logOffset.value, 4096);
    logOffset.value = chunk.nextOffset;
    appendLogChunk(chunk.content);
  } catch {
    // ignore polling errors; next tick may succeed
  }
}

async function startLogPolling() {
  resetLogState();
  logInProgress.value = true;
  logModalOpen.value = true;
  await nextTick();
  ensureLogTerminal();
  applyLogTheme();
  logFitAddon?.fit();
  try {
    const chunk = await readConsoleLog(0, 0);
    logOffset.value = chunk.nextOffset;
  } catch {
    logOffset.value = 0;
  }
  await pollConsoleLog();
  if (logPollTimer !== null) {
    return;
  }
  logPollTimer = window.setInterval(() => {
    void pollConsoleLog();
  }, 800);
}

function stopLogPolling() {
  if (logPollTimer !== null) {
    window.clearInterval(logPollTimer);
    logPollTimer = null;
  }
  logInProgress.value = false;
}

function disposeLogTerminal() {
  if (logResizeObserver) {
    logResizeObserver.disconnect();
    logResizeObserver = null;
  }
  logFitAddon = null;
  logTerminal?.dispose();
  logTerminal = null;
}

async function reloadRemoteBrokersWithLog(message?: string) {
  logContext.value = 'remote-broker';
  logStatusMessage.value = '正在重启远端 broker，请稍候…';
  await startLogPolling();
  try {
    await reloadRemoteBrokers();
    logStatusMessage.value = '远端 broker 重启完成。';
    if (message) {
      showConfigMessage(message);
    }
  } catch (err) {
    const errorMessage = `远端 broker 重启失败：${String(err)}`;
    logStatusMessage.value = errorMessage;
    showConfigMessage(`应用失败：${String(err)}`, 'error');
  } finally {
    stopLogPolling();
  }
}

async function restartConsoleWithLog(message?: string) {
  logContext.value = 'console';
  logStatusMessage.value = '正在重启 console，请稍候…';
  await startLogPolling();
  try {
    await restartConsole();
    logStatusMessage.value = 'console 重启完成。';
    if (message) {
      showConfigMessage(message);
    }
  } catch (err) {
    const msg = `console 重启失败：${String(err)}`;
    logStatusMessage.value = msg;
    showConfigMessage(msg, 'error');
  } finally {
    stopLogPolling();
  }
}

async function loadProfiles(syncSelection = true) {
  const data = await listProfiles();
  profiles.value = data.profiles;
  activeProfile.value = data.current;
  if (syncSelection) {
    selectedProfile.value = data.current;
  } else {
    const keepSelected = data.profiles.some((profile) => profile.name === selectedProfile.value);
    selectedProfile.value = keepSelected ? selectedProfile.value : data.current;
  }
  if (deletableProfileOptions.value.length > 0) {
    if (!deleteProfileName.value) {
      deleteProfileName.value = String(deletableProfileOptions.value[0]?.value ?? '');
    }
  } else {
    deleteProfileName.value = null;
  }
}

async function loadConfigFiles(profileName: string, syncApplied = false) {
  const [proxy, broker] = await Promise.all([
    readProfileProxyConfig(profileName),
    readProfileBrokerConfig(profileName),
  ]);
  proxyConfig.value = proxy;
  brokerConfig.value = broker;
  proxyConfigText.value = proxy.content;
  brokerConfigText.value = broker.content;
  proxyOriginal.value = proxy.content;
  brokerOriginal.value = broker.content;
  if (syncApplied) {
    proxyApplied.value = proxy.content;
    brokerApplied.value = broker.content;
  }
}

async function loadConfigCenter() {
  if (configLoading.value) {
    return;
  }
  configLoading.value = true;
  configError.value = null;
  try {
    await loadProfiles(true);
    if (selectedProfile.value) {
      await loadConfigFiles(selectedProfile.value, true);
    }
    configLoaded.value = true;
  } catch (err) {
    configError.value = String(err);
  } finally {
    configLoading.value = false;
  }
}

function requestProfileChange(value: string | null) {
  if (!value || configBusy.value) {
    return;
  }
  if (proxyDirty.value || brokerDirty.value) {
    pendingProfile.value = value;
    switchProfileOpen.value = true;
    return;
  }
  void applyProfileSelection(value);
}

async function applyProfileSelection(value: string) {
  if (configBusy.value) {
    return;
  }
  const previous = selectedProfile.value;
  configBusy.value = true;
  try {
    selectedProfile.value = value;
    await loadConfigFiles(value);
  } catch (err) {
    selectedProfile.value = previous;
    showConfigMessage(`读取环境配置失败：${String(err)}`, 'error');
  } finally {
    configBusy.value = false;
  }
}

function cancelProfileSwitch() {
  pendingProfile.value = null;
  switchProfileOpen.value = false;
}

function confirmProfileSwitch() {
  const value = pendingProfile.value;
  pendingProfile.value = null;
  switchProfileOpen.value = false;
  if (value) {
    void applyProfileSelection(value);
  }
}

function openCreateProfile() {
  if (proxyDirty.value || brokerDirty.value) {
    showConfigMessage('当前配置有未保存改动，请先保存再新建环境。', 'warning');
    return;
  }
  createProfileName.value = '';
  createProfileOpen.value = true;
}

async function confirmCreateProfile() {
  if (!createProfileValid.value || configBusy.value) {
    return;
  }
  configBusy.value = true;
  try {
    const name = createProfileName.value.trim();
    await createProfile(name);
    await loadProfiles(false);
    selectedProfile.value = name;
    await loadConfigFiles(name);
    showConfigMessage(`已创建环境 ${name}，需要点击应用完成切换。`, 'info');
  } catch (err) {
    showConfigMessage(`新建环境失败：${String(err)}`, 'error');
  } finally {
    configBusy.value = false;
    createProfileOpen.value = false;
    createProfileName.value = '';
  }
}

function openDeleteProfile() {
  if (!canDeleteProfile.value || configBusy.value) {
    return;
  }
  deleteProfileName.value = String(deletableProfileOptions.value[0]?.value ?? '');
  deleteProfileOpen.value = true;
}

async function confirmDeleteProfile() {
  if (!deleteProfileName.value || configBusy.value) {
    return;
  }
  configBusy.value = true;
  try {
    await deleteProfile(deleteProfileName.value);
    await loadProfiles(false);
    if (selectedProfile.value) {
      await loadConfigFiles(selectedProfile.value);
    }
    showConfigMessage(`已删除环境 ${deleteProfileName.value}。`);
  } catch (err) {
    showConfigMessage(`删除环境失败：${String(err)}`, 'error');
  } finally {
    configBusy.value = false;
    deleteProfileOpen.value = false;
    deleteProfileName.value = null;
  }
}

function requestRefreshConfig() {
  if (configBusy.value || configLoading.value) {
    return;
  }
  if (proxyDirty.value || brokerDirty.value) {
    refreshConfirmOpen.value = true;
    return;
  }
  void refreshConfigNow();
}

async function refreshConfigNow() {
  configBusy.value = true;
  try {
    const previousSelection = selectedProfile.value;
    await loadProfiles(false);
    const profileName = selectedProfile.value ?? previousSelection;
    if (profileName) {
      await loadConfigFiles(profileName);
    }
    showConfigMessage('已刷新配置。');
  } catch (err) {
    showConfigMessage(`刷新配置失败：${String(err)}`, 'error');
  } finally {
    configBusy.value = false;
    refreshConfirmOpen.value = false;
  }
}

function cancelRefreshConfirm() {
  refreshConfirmOpen.value = false;
}

function confirmRefresh() {
  void refreshConfigNow();
}

async function saveConfigFiles() {
  if (configBusy.value || configLoading.value) {
    return;
  }
  const profileName = selectedProfile.value;
  if (!profileName) {
    showConfigMessage('请先选择环境。', 'warning');
    return;
  }
  if (!proxyDirty.value && !brokerDirty.value) {
    showConfigMessage('配置未改动。', 'info');
    return;
  }
  configBusy.value = true;
  try {
    const tasks: Promise<void>[] = [];
    if (proxyDirty.value) {
      tasks.push(writeProfileProxyConfig(profileName, proxyConfigText.value));
    }
    if (brokerDirty.value) {
      tasks.push(writeProfileBrokerConfig(profileName, brokerConfigText.value));
    }
    await Promise.all(tasks);
    proxyOriginal.value = proxyConfigText.value;
    brokerOriginal.value = brokerConfigText.value;
    showConfigMessage('配置已保存。');
  } catch (err) {
    showConfigMessage(`保存配置失败：${String(err)}`, 'error');
  } finally {
    configBusy.value = false;
  }
}

function requestApplyConfig() {
  if (configBusy.value || configLoading.value) {
    return;
  }
  if (!selectedProfile.value) {
    showConfigMessage('请先选择环境。', 'warning');
    return;
  }
  if (proxyDirty.value || brokerDirty.value) {
    showConfigMessage('配置有未保存改动，请先保存再应用。', 'warning');
    return;
  }
  const switching = selectedProfile.value !== activeProfile.value;
  const brokerChanged =
    brokerApplied.value !== null && brokerApplied.value !== brokerConfigText.value;
  if (!switching && brokerChanged) {
    confirmApplyOpen.value = true;
    return;
  }
  void applyConfig();
}

function cancelApplyConfirm() {
  confirmApplyOpen.value = false;
}

function confirmApply() {
  confirmApplyOpen.value = false;
  void applyConfig();
}

async function applyConfig() {
  if (configBusy.value || configLoading.value) {
    return;
  }
  const profileName = selectedProfile.value;
  if (!profileName) {
    showConfigMessage('请先选择环境。', 'warning');
    return;
  }
  if (proxyDirty.value || brokerDirty.value) {
    showConfigMessage('配置有未保存改动，请先保存再应用。', 'warning');
    return;
  }
  const switching = profileName !== activeProfile.value;
  const proxyChanged = proxyApplied.value !== null && proxyApplied.value !== proxyConfigText.value;
  const brokerChanged = brokerApplied.value !== null && brokerApplied.value !== brokerConfigText.value;
  const shouldRestartConsole = switching || proxyChanged;
  const shouldReloadRemoteBrokers = brokerChanged;
  configBusy.value = true;
  try {
    if (switching) {
      await selectProfile(profileName);
      activeProfile.value = profileName;
    }
    if (shouldRestartConsole) {
      const message = shouldReloadRemoteBrokers
        ? undefined
        : switching
          ? `已切换到环境 ${profileName}。`
          : '本地配置已应用，console 已重启。';
      await restartConsoleWithLog(message);
      proxyApplied.value = proxyConfigText.value;
    }
    if (shouldReloadRemoteBrokers) {
      const message = switching
        ? `已切换到环境 ${profileName}，远端 broker 已重启。`
        : '远端配置已应用，远端 broker 已重启。';
      await reloadRemoteBrokersWithLog(message);
      brokerApplied.value = brokerConfigText.value;
    } else if (switching) {
      brokerApplied.value = brokerConfigText.value;
    }
    if (!shouldRestartConsole && !shouldReloadRemoteBrokers) {
      showConfigMessage('配置未改动。', 'info');
    }
  } catch (err) {
    showConfigMessage(`应用失败：${String(err)}`, 'error');
  } finally {
    configBusy.value = false;
  }
}

function closeLogModal() {
  if (logInProgress.value) {
    return;
  }
  logModalOpen.value = false;
}

function captureShortcut(field: ShortcutField, event: KeyboardEvent) {
  if (event.code === 'Escape') {
    activeShortcut.value = null;
    emit('close');
    return;
  }
  const shortcut = eventToShortcut(event);
  if (!shortcut) {
    return;
  }
  const normalized = normalizeShortcut(shortcut);
  if (!normalized) {
    return;
  }
  localSettings.value.shortcuts[field] = normalized;
}

function shortcutDisplay(field: ShortcutField) {
  const formatted = formatShortcut(localSettings.value.shortcuts[field]);
  if (activeShortcut.value === field) {
    return formatted || '按键盘设置快捷键';
  }
  return formatted || '点击设置快捷键';
}

function shortcutHasValue(field: ShortcutField) {
  return Boolean(formatShortcut(localSettings.value.shortcuts[field]));
}

function shortcutIsDefault(field: ShortcutField) {
  const current = normalizeShortcut(localSettings.value.shortcuts[field] ?? '') ?? '';
  const fallback = normalizeShortcut(DEFAULT_SETTINGS.shortcuts[field] ?? '') ?? '';
  return current === fallback;
}

function shortcutInputClass(field: ShortcutField) {
  if (activeShortcut.value === field) {
    return 'border-accent text-accent ring-1 ring-accent/60';
  }
  if (shortcutHasValue(field)) {
    return 'border-border text-foreground';
  }
  return 'border-border text-foreground-muted';
}

function clearShortcut(field: ShortcutField) {
  localSettings.value.shortcuts[field] = '';
}

function resetShortcut(field: ShortcutField) {
  localSettings.value.shortcuts[field] = DEFAULT_SETTINGS.shortcuts[field];
}

function activateShortcut(field: ShortcutField) {
  activeShortcut.value = field;
}

function deactivateShortcut(field: ShortcutField) {
  if (activeShortcut.value === field) {
    activeShortcut.value = null;
  }
}

onBeforeUnmount(() => {
  if (configMessageTimer !== null) {
    window.clearTimeout(configMessageTimer);
  }
  stopLogPolling();
  disposeLogTerminal();
  stopCardObserver();
});

watch(
  () => activeTab.value,
  (tab) => {
    if (tab === 'config' && hasOpen.value && !configLoaded.value) {
      void loadConfigCenter();
    }
    void syncCardHeight();
  }
);

watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      localSettings.value = cloneSettings(props.settings);
      activeShortcut.value = null;
      cardHeight.value = null;
      cardShellReady.value = false;
      void syncCardHeight();
      return;
    }
    if (!open) {
      configLoaded.value = false;
      configError.value = null;
      configMessage.value = null;
      configMessageType.value = 'success';
      confirmApplyOpen.value = false;
      logModalOpen.value = false;
      logStatusMessage.value = '';
      logContext.value = 'remote-broker';
      profiles.value = [];
      activeProfile.value = null;
      selectedProfile.value = null;
      pendingProfile.value = null;
      switchProfileOpen.value = false;
      createProfileOpen.value = false;
      createProfileName.value = '';
      deleteProfileOpen.value = false;
      deleteProfileName.value = null;
      refreshConfirmOpen.value = false;
      proxyConfig.value = null;
      brokerConfig.value = null;
      proxyConfigText.value = '';
      brokerConfigText.value = '';
      proxyOriginal.value = '';
      brokerOriginal.value = '';
      proxyApplied.value = null;
      brokerApplied.value = null;
      logHasOutput.value = false;
      logOffset.value = 0;
      activeTab.value = 'general';
      activeShortcut.value = null;
      cardHeight.value = null;
      cardShellReady.value = false;
      stopCardObserver();
    }
  }
);

watch(
  () => logModalOpen.value,
  (open) => {
    if (!open) {
      stopLogPolling();
      disposeLogTerminal();
    }
  }
);

watch(
  () => props.resolvedTheme,
  () => {
    applyLogTheme();
  }
);

watch(
  () => [configLoading.value, configError.value, configMessage.value],
  () => {
    if (activeTab.value === 'config') {
      void syncCardHeight();
    }
  }
);
</script>

<template>
  <n-modal
    :show="hasOpen"
    :mask-closable="false"
    :close-on-esc="!logModalOpen"
    @update:show="(value) => { if (!value && !logModalOpen) emit('close'); }"
  >
    <div
      ref="cardShellRef"
      class="settings-card-shell"
      :class="{ 'settings-card-shell--ready': cardShellReady }"
      :style="cardShellStyle"
    >
      <div ref="cardInnerRef" class="w-full">
        <n-card :bordered="true" :style="cardStyle" :content-style="cardContentStyle" size="large">
          <template #header>设置</template>
          <template #header-extra>
            <n-button text :disabled="logModalOpen" @click="emit('close')" aria-label="关闭" title="关闭">
              <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                <line x1="18" y1="6" x2="6" y2="18" />
                <line x1="6" y1="6" x2="18" y2="18" />
              </svg>
            </n-button>
          </template>

          <n-tabs
            v-model:value="activeTab"
            type="line"
            size="small"
            :class="isConfigTab ? 'settings-tabs settings-tabs--full' : 'settings-tabs'"
          >
            <n-tab-pane name="general" tab="通用设置">
              <div class="space-y-4">
                <div class="flex items-center justify-between gap-4">
                  <div>
                    <div class="text-sm font-medium">主题</div>
                    <div class="text-xs text-foreground-muted">系统/深色/浅色</div>
                  </div>
                  <n-select
                    v-model:value="localSettings.theme"
                    :options="themeOptions"
                    size="small"
                    class="w-32"
                  />
                </div>
                <div class="flex items-center justify-between">
                  <div>
                    <div class="text-sm font-medium">新 Pending 通知</div>
                    <div class="text-xs text-foreground-muted">有新的待审批时弹出提示</div>
                  </div>
                  <n-switch v-model:value="localSettings.notificationsEnabled" size="small" />
                </div>
              </div>
            </n-tab-pane>

            <n-tab-pane name="shortcuts" tab="快捷键设置">
              <div class="space-y-3">
                <div
                  v-for="item in shortcutFields"
                  :key="item.key"
                  class="flex items-center justify-between gap-4 text-sm"
                >
                  <span class="text-foreground-muted">{{ item.label }}</span>
                  <div class="flex items-center gap-2">
                    <div class="w-[120px] flex-none">
                      <n-input
                        :value="shortcutDisplay(item.key)"
                        size="small"
                        readonly
                        class="w-full"
                        :class="shortcutInputClass(item.key)"
                        @focus="activateShortcut(item.key)"
                        @blur="deactivateShortcut(item.key)"
                        @keydown.prevent="captureShortcut(item.key, $event)"
                      />
                    </div>
                    <n-button
                      size="small"
                      quaternary
                      :disabled="!shortcutHasValue(item.key)"
                      title="清空"
                      aria-label="清空快捷键"
                      @click="clearShortcut(item.key)"
                    >
                      <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                        <line x1="18" y1="6" x2="6" y2="18" />
                        <line x1="6" y1="6" x2="18" y2="18" />
                      </svg>
                    </n-button>
                    <n-button
                      size="small"
                      quaternary
                      :disabled="shortcutIsDefault(item.key)"
                      title="恢复默认"
                      aria-label="恢复默认快捷键"
                      @click="resetShortcut(item.key)"
                    >
                      <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M3 12a9 9 0 1 0 3-6.7" />
                        <polyline points="3 4 3 10 9 10" />
                      </svg>
                    </n-button>
                  </div>
                </div>
              </div>
            </n-tab-pane>

            <n-tab-pane name="ai" tab="AI 检查">
              <div class="grid grid-cols-1 lg:grid-cols-[minmax(0,1fr)_minmax(0,360px)] gap-6 items-start">
                <div class="space-y-4">
                  <div class="ai-field">
                    <div>
                      <div class="text-sm font-medium">启用 AI 检查</div>
                      <div class="text-xs text-foreground-muted">对所有 Pending 命令进行风险评估</div>
                    </div>
                    <div class="ai-control ai-control--switch">
                      <n-switch v-model:value="localSettings.ai.enabled" size="small" />
                    </div>
                  </div>

                  <div class="space-y-3">
                    <div class="ai-field">
                      <div>
                        <div class="text-sm font-medium">Provider</div>
                        <div class="text-xs text-foreground-muted">兼容 OpenAI 的接口</div>
                      </div>
                      <div class="ai-control">
                        <n-select v-model:value="localSettings.ai.provider" :options="aiProviderOptions" size="small" class="w-full" />
                      </div>
                    </div>

                    <div class="ai-field">
                      <div>
                        <div class="text-sm font-medium">Base URL</div>
                        <div class="text-xs text-foreground-muted">模型服务地址</div>
                      </div>
                      <div class="ai-control">
                        <n-input v-model:value="localSettings.ai.baseUrl" size="small" class="w-full" />
                      </div>
                    </div>

                    <div class="ai-field">
                      <div>
                        <div class="text-sm font-medium">Chat Path</div>
                        <div class="text-xs text-foreground-muted">请求路径</div>
                      </div>
                      <div class="ai-control">
                        <n-input v-model:value="localSettings.ai.chatPath" size="small" class="w-full" />
                      </div>
                    </div>

                    <div class="ai-field">
                      <div>
                        <div class="text-sm font-medium">Model</div>
                        <div class="text-xs text-foreground-muted">模型名称</div>
                      </div>
                      <div class="ai-control">
                        <n-input v-model:value="localSettings.ai.model" size="small" class="w-full" />
                      </div>
                    </div>

                    <div class="ai-field">
                      <div>
                        <div class="text-sm font-medium">API Key</div>
                        <div class="text-xs text-foreground-muted">仅保存在本地设置</div>
                      </div>
                      <div class="ai-control">
                        <n-input v-model:value="localSettings.ai.apiKey" size="small" class="w-full" type="password" />
                      </div>
                    </div>

                    <div class="ai-field">
                      <div>
                        <div class="text-sm font-medium">Timeout (ms)</div>
                        <div class="text-xs text-foreground-muted">超时后视为失败</div>
                      </div>
                      <div class="ai-control">
                        <n-input-number
                          :value="localSettings.ai.timeoutMs"
                          size="small"
                          class="w-full"
                          :min="1000"
                          :max="60000"
                          @update:value="(value) => { localSettings.ai.timeoutMs = value ?? DEFAULT_SETTINGS.ai.timeoutMs; }"
                        />
                      </div>
                    </div>

                    <div class="ai-field">
                      <div>
                        <div class="text-sm font-medium">最大并发</div>
                        <div class="text-xs text-foreground-muted">同时评估的请求数</div>
                      </div>
                      <div class="ai-control">
                        <n-input-number
                          :value="localSettings.ai.maxConcurrency"
                          size="small"
                          class="w-full"
                          :min="1"
                          :max="10"
                          @update:value="(value) => { localSettings.ai.maxConcurrency = value ?? DEFAULT_SETTINGS.ai.maxConcurrency; }"
                        />
                      </div>
                    </div>
                  </div>
                </div>

                <div class="flex flex-col gap-2">
                  <div class="flex items-start justify-between gap-3">
                    <div>
                      <div class="text-sm font-medium">Prompt</div>
                      <div class="text-xs text-foreground-muted" v-pre>支持 {{field}} 占位</div>
                    </div>
                    <n-button size="small" quaternary @click="localSettings.ai.prompt = DEFAULT_SETTINGS.ai.prompt">
                      恢复默认
                    </n-button>
                  </div>
                  <div class="ai-control ai-control--prompt">
                    <n-input
                      v-model:value="localSettings.ai.prompt"
                      type="textarea"
                      class="w-full"
                      :autosize="{ minRows: 16, maxRows: 16 }"
                    />
                  </div>
                </div>
              </div>
            </n-tab-pane>

            <n-tab-pane name="config" tab="配置中心">
              <div class="flex flex-col gap-4 min-h-0 flex-1">
                <div v-if="configLoading" class="flex items-center gap-2 text-sm text-foreground-muted">
                  <n-spin size="small" />
                  <span>正在加载配置...</span>
                </div>
                <n-alert v-else-if="configError" type="error" :bordered="false">
                  加载失败：{{ configError }}
                </n-alert>
                <div v-else class="flex flex-col gap-3 min-h-0 flex-1">
                  <div class="flex flex-wrap items-center justify-between gap-3 text-sm">
                    <div>
                      <div class="font-medium">环境</div>
                      <div class="text-xs text-foreground-muted">选择需要编辑的配置</div>
                    </div>
                    <div class="flex items-center gap-2">
                      <n-select
                        :value="selectedProfile"
                        :options="profileOptions"
                        size="small"
                        class="w-40"
                        placeholder="请选择环境"
                        :disabled="configBusy || logModalOpen || configLoading"
                        @update:value="requestProfileChange"
                      />
                      <n-button size="small" :disabled="configBusy || logModalOpen || configLoading" @click="openCreateProfile">
                        新建
                      </n-button>
                      <n-button
                        size="small"
                        quaternary
                        :disabled="configBusy || logModalOpen || configLoading || !canDeleteProfile"
                        @click="openDeleteProfile"
                      >
                        删除
                      </n-button>
                    </div>
                  </div>

                  <div class="grid grid-cols-1 lg:grid-cols-2 gap-4 min-h-0 flex-1">
                    <div class="flex flex-col gap-2 min-h-0 flex-1">
                      <div class="flex items-center justify-between text-sm">
                        <div>
                          <div class="font-medium">本地代理配置</div>
                          <div class="text-xs text-foreground-muted break-all">{{ proxyConfig?.path }}</div>
                        </div>
                        <span v-if="proxyConfig && !proxyConfig.exists" class="text-xs text-warning">未创建</span>
                      </div>
                      <div class="flex-1 min-h-0">
                        <MonacoEditor v-model="proxyConfigText" language="toml" height="100%" :theme="props.resolvedTheme" />
                      </div>
                    </div>

                    <div class="flex flex-col gap-2 min-h-0 flex-1">
                      <div class="flex items-center justify-between text-sm">
                        <div>
                          <div class="font-medium">远端 broker 配置（源文件）</div>
                          <div class="text-xs text-foreground-muted break-all">{{ brokerConfig?.path }}</div>
                        </div>
                      </div>
                      <div class="flex-1 min-h-0">
                        <MonacoEditor v-model="brokerConfigText" language="toml" height="100%" :theme="props.resolvedTheme" />
                      </div>
                    </div>
                  </div>
                </div>

                <n-alert v-if="configMessage" :type="configMessageType" :bordered="false">
                  {{ configMessage }}
                </n-alert>
              </div>
            </n-tab-pane>
          </n-tabs>

          <div v-if="activeTab !== 'config'" class="mt-6 flex justify-end gap-3">
            <n-button @click="emit('close')">取消</n-button>
            <n-button type="primary" @click="save">保存</n-button>
          </div>

          <div v-else class="mt-4 flex items-center justify-between gap-3">
            <div class="text-xs text-foreground-muted">
              当前环境 {{ activeProfile || '-' }} · 已选择 {{ selectedProfile || '-' }} · 本地配置
              {{ proxyDirty ? '有改动' : '未改动' }} · 远端配置 {{ brokerDirty ? '有改动' : '未改动' }}
            </div>
            <div class="flex items-center gap-2">
              <n-button
                quaternary
                :disabled="configBusy || logModalOpen || configLoading"
                @click="requestRefreshConfig"
              >
                刷新
              </n-button>
              <n-button :disabled="configBusy || logModalOpen" @click="emit('close')">取消</n-button>
              <n-button :disabled="configBusy || logModalOpen || configLoading" @click="saveConfigFiles">保存</n-button>
              <n-button
                type="primary"
                :disabled="configBusy || logModalOpen || configLoading"
                @click="requestApplyConfig"
              >
                应用
              </n-button>
            </div>
          </div>
        </n-card>
      </div>
    </div>
  </n-modal>

  <n-modal v-model:show="confirmApplyOpen" :mask-closable="false" :close-on-esc="true">
    <n-card size="small" class="w-[22rem]" :bordered="true">
      <template #header>确认应用</template>
      <div class="text-sm text-foreground-muted">远端配置应用会导致重新连接，请确认。</div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button @click="cancelApplyConfirm">取消</n-button>
          <n-button type="primary" :disabled="configBusy" @click="confirmApply">继续应用</n-button>
        </div>
      </template>
    </n-card>
  </n-modal>

  <n-modal v-model:show="switchProfileOpen" :mask-closable="false" :close-on-esc="true">
    <n-card size="small" class="w-[22rem]" :bordered="true">
      <template #header>切换配置</template>
      <div class="text-sm text-foreground-muted">当前配置有未保存改动，切换会丢失，是否继续？</div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button @click="cancelProfileSwitch">取消</n-button>
          <n-button type="primary" :disabled="configBusy" @click="confirmProfileSwitch">
            放弃改动并切换
          </n-button>
        </div>
      </template>
    </n-card>
  </n-modal>

  <n-modal v-model:show="createProfileOpen" :mask-closable="false" :close-on-esc="true">
    <n-card size="small" class="w-[22rem]" :bordered="true">
      <template #header>新建环境</template>
      <div class="space-y-2">
        <n-input v-model:value="createProfileName" size="small" placeholder="例如 dev" />
        <div class="text-xs text-foreground-muted">名称仅支持字母、数字、- 或 _</div>
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button @click="createProfileOpen = false">取消</n-button>
          <n-button type="primary" :disabled="!createProfileValid || configBusy" @click="confirmCreateProfile">
            创建
          </n-button>
        </div>
      </template>
    </n-card>
  </n-modal>

  <n-modal v-model:show="deleteProfileOpen" :mask-closable="false" :close-on-esc="true">
    <n-card size="small" class="w-[22rem]" :bordered="true">
      <template #header>删除环境</template>
      <div class="space-y-2">
        <n-select
          v-model:value="deleteProfileName"
          :options="deletableProfileOptions"
          size="small"
          placeholder="选择要删除的环境"
        />
        <div class="text-xs text-warning">删除后无法恢复，请谨慎操作。</div>
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button @click="deleteProfileOpen = false">取消</n-button>
          <n-button type="error" :disabled="!deleteProfileName || configBusy" @click="confirmDeleteProfile">
            确认删除
          </n-button>
        </div>
      </template>
    </n-card>
  </n-modal>

  <n-modal v-model:show="refreshConfirmOpen" :mask-closable="false" :close-on-esc="true">
    <n-card size="small" class="w-[22rem]" :bordered="true">
      <template #header>刷新配置</template>
      <div class="text-sm text-foreground-muted">当前配置有未保存改动，刷新会丢失，是否继续？</div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button @click="cancelRefreshConfirm">取消</n-button>
          <n-button type="primary" :disabled="configBusy" @click="confirmRefresh">继续刷新</n-button>
        </div>
      </template>
    </n-card>
  </n-modal>

  <n-modal v-model:show="logModalOpen" :mask-closable="false" :close-on-esc="false">
    <n-card size="small" class="w-[36rem]" :bordered="true">
      <template #header>{{ logTitle }}</template>
      <div class="text-sm text-foreground-muted">
        {{ logStatusMessage || '正在准备日志...' }}
      </div>
      <div class="relative mt-3 h-64 overflow-hidden rounded border border-border bg-panel-muted">
        <div ref="logTerminalRef" class="h-full w-full" />
        <div
          v-if="!logHasOutput"
          class="absolute inset-0 flex items-center justify-center text-xs text-foreground-muted pointer-events-none"
        >
          暂无日志输出
        </div>
      </div>
      <template #footer>
        <div class="flex items-center justify-between gap-3">
          <div class="flex items-center gap-2 text-xs text-foreground-muted">
            <n-spin v-if="logInProgress" size="small" />
            <span>{{ logFooterText }}</span>
          </div>
          <n-button type="primary" :disabled="logInProgress" @click="closeLogModal">完成</n-button>
        </div>
      </template>
    </n-card>
  </n-modal>
</template>

<style scoped>
.settings-card-shell {
  width: 100%;
  overflow: hidden;
}

.settings-card-shell--ready {
  transition: max-width 320ms cubic-bezier(0.22, 1, 0.36, 1),
    height 320ms cubic-bezier(0.22, 1, 0.36, 1);
  will-change: max-width, height;
}

.settings-tabs--full {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
}

.settings-tabs--full :deep(.n-tabs-content),
.settings-tabs--full :deep(.n-tabs-pane-wrapper),
.settings-tabs--full :deep(.n-tab-pane) {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.ai-field {
  display: grid;
  gap: 0.75rem;
  align-items: center;
  height: 2.5rem;
}

@media (min-width: 640px) {
  .ai-field {
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
  }
}

.ai-field > div:first-child {
  min-height: 2.5rem;
  display: flex;
  flex-direction: column;
  justify-content: center;
}

.ai-field > div:first-child .text-xs {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  overflow: hidden;
}

.ai-control {
  width: 100%;
}

@media (min-width: 640px) {
  .ai-control {
    width: 16rem;
    display: flex;
    justify-content: flex-end;
  }
}

.ai-control--switch {
  display: flex;
  justify-content: flex-end;
}

.ai-control--prompt {
  width: 100%;
  display: block;
}

.ai-control :deep(.n-input__input input),
.ai-control :deep(.n-input__textarea textarea),
.ai-control :deep(.n-base-selection-label) {
  color: rgb(var(--color-text));
}

.ai-control :deep(.n-input__input input::placeholder),
.ai-control :deep(.n-input__textarea textarea::placeholder),
.ai-control :deep(.n-base-selection-placeholder) {
  color: rgb(var(--color-text-muted));
}
</style>
