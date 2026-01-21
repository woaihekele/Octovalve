<script setup lang="ts">
import { computed, inject, nextTick, onBeforeUnmount, ref, watch, type CSSProperties } from 'vue';
import { Terminal, type ITheme } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import {
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
  useNotification,
  type SelectOption,
} from 'naive-ui';
import { useI18n } from 'vue-i18n';
import {
  createProfile,
  deleteProfile,
  listProfiles,
  readConsoleLog,
  readProfileBrokerConfig,
  readProfileProxyConfig,
  restartConsole,
  selectProfile,
  writeProfileBrokerConfig,
  writeProfileProxyConfig,
} from '../../services/api';
import { loadSettings } from '../../services/settings';
import type { AppSettings, ConfigFilePayload, ProfileSummary, ThemeMode } from '../../shared/types';
import { type ResolvedTheme } from '../../shared/theme';
import { APPLY_THEME_MODE } from '../../app/appContext';
import { AiInspectionSettings, ChatProviderSettings, ConfigCenterSettings, GeneralSettings, ShortcutsSettings } from './settings';
import type { ChatProviderConfig } from '../../shared/types';

const props = defineProps<{
  isOpen: boolean;
  settings: AppSettings;
  resolvedTheme: ResolvedTheme;
  focusConfigToken?: number;
}>();

const emit = defineEmits<{
  (e: 'close'): void;
  (e: 'save', settings: AppSettings, close?: boolean): void;
  (e: 'preview', key: 'language' | 'uiScale' | 'terminalScale', value: unknown): void;
}>();

const applyThemeMode = inject(APPLY_THEME_MODE, () => {});

function cloneSettings(source: AppSettings): AppSettings {
  return {
    notificationsEnabled: source.notificationsEnabled,
    theme: source.theme,
    language: source.language,
    uiScale: source.uiScale,
    terminalScale: source.terminalScale,
    ai: { ...source.ai },
    chat: {
      provider: source.chat.provider,
      sendOnEnter: source.chat.sendOnEnter,
      mcpConfigJson: source.chat.mcpConfigJson,
      openai: { ...source.chat.openai },
      acp: { ...source.chat.acp },
    },
    shortcuts: { ...source.shortcuts },
  };
}

const localSettings = ref<AppSettings>(cloneSettings(props.settings));
const activeTab = ref<'general' | 'shortcuts' | 'chat' | 'ai' | 'config'>('general');
const initialTheme = ref<ThemeMode>(props.settings.theme);
const { t } = useI18n();
const configLoading = ref(false);
const configBusy = ref(false);
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
const logStatusMessage = ref('');
const logHasOutput = ref(false);
const logOffset = ref(0);
const logTerminalRef = ref<HTMLDivElement | null>(null);
const LOG_BASE_FONT_SIZE = 12;
let logPollTimer: number | null = null;
let logTerminal: Terminal | null = null;
let logFitAddon: FitAddon | null = null;
let logResizeObserver: ResizeObserver | null = null;
const notification = useNotification();
const cardShellRef = ref<HTMLDivElement | null>(null);
const cardInnerRef = ref<HTMLDivElement | null>(null);
const cardHeight = ref<number | null>(null);
const cardShellReady = ref(false);
const highlightConfig = ref(false);
let cardResizeObserver: ResizeObserver | null = null;
let highlightTimer: number | null = null;

watch(
  () => props.settings,
  (value) => {
    localSettings.value = cloneSettings(value);
  },
  { deep: true }
);

watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      initialTheme.value = props.settings.theme;
      return;
    }
    if (initialTheme.value !== props.settings.theme) {
      initialTheme.value = props.settings.theme;
    }
    applyThemeMode(props.settings.theme);
    localSettings.value = cloneSettings(props.settings);
  }
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
const logTitle = computed(() => t('settings.log.title.console'));
const logFooterText = computed(() =>
  logInProgress.value ? t('settings.log.footer.console.pending') : t('settings.log.footer.console.done')
);
const configStatusText = computed(() =>
  t('settings.config.status', {
    active: activeProfile.value || '-',
    selected: selectedProfile.value || '-',
    proxy: proxyDirty.value ? t('settings.config.changed') : t('settings.config.unchanged'),
    broker: brokerDirty.value ? t('settings.config.changed') : t('settings.config.unchanged'),
  })
);
const isAiTab = computed(() => activeTab.value === 'ai');
const isChatTab = computed(() => activeTab.value === 'chat');
const cardMaxWidth = computed(() =>
  '56rem'
);
const cardShellStyle = computed<CSSProperties>(() => ({
  width: cardMaxWidth.value,
  maxWidth: '100%',
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
    minHeight: 0,
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
  const measuredHeight = isConfigTab.value ? inner.offsetHeight : Math.max(inner.scrollHeight, inner.offsetHeight);
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

function clearConfigHighlight() {
  if (highlightTimer !== null) {
    window.clearTimeout(highlightTimer);
    highlightTimer = null;
  }
  highlightConfig.value = false;
}

function triggerConfigHighlight() {
  highlightConfig.value = true;
  if (highlightTimer !== null) {
    window.clearTimeout(highlightTimer);
  }
  highlightTimer = window.setTimeout(() => {
    highlightConfig.value = false;
    highlightTimer = null;
  }, 4000);
}

function save(close?: boolean | MouseEvent) {
  const shouldClose = typeof close === 'boolean' ? close : true;
  emit('save', cloneSettings(localSettings.value), shouldClose);
}

function updateSetting(key: keyof AppSettings, value: unknown) {
  (localSettings.value as Record<string, unknown>)[key] = value;
  if (key === 'theme') {
    applyThemeMode(value as ThemeMode);
  }
  if (key === 'language' || key === 'uiScale' || key === 'terminalScale') {
    emit('preview', key, value);
  }
}

function updateShortcut(key: string, value: string) {
  (localSettings.value.shortcuts as Record<string, string>)[key] = value;
}

function showConfigMessage(message: string, type: 'success' | 'error' | 'warning' | 'info' = 'success') {
  notification.create({
    title: message,
    duration: 4000,
    type,
  });
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

function resolveLogTheme(): ITheme {
  return {
    background: resolveRgbVar('--color-panel-muted', '30 41 59'),
    foreground: resolveRgbVar('--color-text', '226 232 240'),
    cursor: resolveRgbVar('--color-accent', '99 102 241'),
    selectionBackground: resolveRgbaVar('--color-accent', '99 102 241', 0.35),
  };
}

function resolveLogFontSize() {
  return LOG_BASE_FONT_SIZE * (localSettings.value.terminalScale || 1);
}

function ensureLogTerminal() {
  if (logTerminal || !logTerminalRef.value) {
    return;
  }
  logTerminal = new Terminal({
    disableStdin: true,
    convertEol: true,
    fontSize: resolveLogFontSize(),
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

function applyLogScale() {
  if (!logTerminal) {
    return;
  }
  logTerminal.options.fontSize = resolveLogFontSize();
  logFitAddon?.fit();
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

async function restartConsoleWithLog(message?: string): Promise<boolean> {
  logStatusMessage.value = t('settings.log.status.console.pending');
  await startLogPolling();
  try {
    await restartConsole();
    logStatusMessage.value = t('settings.log.status.console.done');
    if (message) {
      showConfigMessage(message);
    }
    return true;
  } catch (err) {
    const msg = t('settings.log.status.console.failed', { error: String(err) });
    logStatusMessage.value = msg;
    showConfigMessage(msg, 'error');
    return false;
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
  try {
    await loadProfiles(true);
    if (selectedProfile.value) {
      await loadConfigFiles(selectedProfile.value, true);
    }
    configLoaded.value = true;
  } catch (err) {
    showConfigMessage(t('settings.config.loadFailed', { error: String(err) }), 'error');
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
    showConfigMessage(t('settings.profile.loadFailed', { error: String(err) }), 'error');
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
    showConfigMessage(t('settings.profile.createBlocked'), 'warning');
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
    showConfigMessage(t('settings.profile.created', { name }), 'info');
  } catch (err) {
    showConfigMessage(t('settings.profile.createFailed', { error: String(err) }), 'error');
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
    showConfigMessage(t('settings.profile.deleted', { name: deleteProfileName.value }));
  } catch (err) {
    showConfigMessage(t('settings.profile.deleteFailed', { error: String(err) }), 'error');
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
    showConfigMessage(t('settings.config.refreshed'));
  } catch (err) {
    showConfigMessage(t('settings.config.refreshFailed', { error: String(err) }), 'error');
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

  save(false);

  const profileName = selectedProfile.value;
  if (!profileName) {
    showConfigMessage(t('settings.profile.selectFirst'), 'warning');
    return;
  }
  if (!proxyDirty.value && !brokerDirty.value) {
    showConfigMessage(t('settings.config.noChanges'), 'info');
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
    showConfigMessage(t('settings.config.saved'));
  } catch (err) {
    showConfigMessage(t('settings.config.saveFailed', { error: String(err) }), 'error');
  } finally {
    configBusy.value = false;
  }
}

function requestApplyConfig() {
  if (configBusy.value || configLoading.value) {
    return;
  }

  save(false);

  if (!selectedProfile.value) {
    showConfigMessage(t('settings.profile.selectFirst'), 'warning');
    return;
  }
  void applyConfig();
}

async function applyConfig() {
  if (configBusy.value || configLoading.value) {
    return;
  }
  const profileName = selectedProfile.value;
  if (!profileName) {
    showConfigMessage(t('settings.profile.selectFirst'), 'warning');
    return;
  }

  configBusy.value = true;
  let success = false;
  try {
    // Auto-save dirty config files before applying
    if (proxyDirty.value || brokerDirty.value) {
      const saveTasks: Promise<void>[] = [];
      if (proxyDirty.value) {
        saveTasks.push(writeProfileProxyConfig(profileName, proxyConfigText.value));
      }
      if (brokerDirty.value) {
        saveTasks.push(writeProfileBrokerConfig(profileName, brokerConfigText.value));
      }
      await Promise.all(saveTasks);
      proxyOriginal.value = proxyConfigText.value;
      brokerOriginal.value = brokerConfigText.value;
    }

    const switching = profileName !== activeProfile.value;
    const proxyChanged = proxyApplied.value !== null && proxyApplied.value !== proxyConfigText.value;
    const brokerChanged = brokerApplied.value !== null && brokerApplied.value !== brokerConfigText.value;
    const shouldRestartConsole = switching || proxyChanged || brokerChanged;

    if (switching) {
      await selectProfile(profileName);
      activeProfile.value = profileName;
    }
    if (shouldRestartConsole) {
      const message = switching
        ? t('settings.apply.switchProfile', { name: profileName })
        : t('settings.apply.localApplied');
      success = await restartConsoleWithLog(message);
      proxyApplied.value = proxyConfigText.value;
      brokerApplied.value = brokerConfigText.value;
    }
    if (!shouldRestartConsole) {
      showConfigMessage(t('settings.config.noChanges'), 'info');
      success = true;
    }
  } catch (err) {
    showConfigMessage(t('settings.apply.failed', { error: String(err) }), 'error');
  } finally {
    configBusy.value = false;
    if (success) {
      logModalOpen.value = false;
      emit('close');
    }
  }
}

function closeLogModal() {
  if (logInProgress.value) {
    return;
  }
  logModalOpen.value = false;
}

onBeforeUnmount(() => {
  clearConfigHighlight();
  stopLogPolling();
  disposeLogTerminal();
  stopCardObserver();
});

watch(
  () => props.focusConfigToken,
  (token, prev) => {
    if (token === undefined || token === null || token === prev) {
      return;
    }
    activeTab.value = 'config';
    triggerConfigHighlight();
  }
);

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
      cardHeight.value = null;
      cardShellReady.value = false;
      void syncCardHeight();
      return;
    }
    if (!open) {
      clearConfigHighlight();
      configLoaded.value = false;
      logModalOpen.value = false;
      logStatusMessage.value = '';
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
  () => localSettings.value.terminalScale,
  () => {
    applyLogScale();
  }
);

watch(
  () => [configLoading.value],
  () => {
    if (activeTab.value === 'config') {
      void syncCardHeight();
    }
  }
);
</script>

<template>
  <n-modal
    class="settings-modal"
    :show="hasOpen"
    :mask-closable="false"
    :close-on-esc="!logModalOpen"
    @update:show="(value) => { if (!value && !logModalOpen) emit('close'); }"
  >
    <div class="settings-modal-root">
      <div
        ref="cardShellRef"
        class="settings-card-shell"
        :class="{ 'settings-card-shell--ready': cardShellReady }"
        :style="cardShellStyle"
      >
        <div ref="cardInnerRef" class="w-full">
          <n-card
            :bordered="true"
            :style="cardStyle"
            :content-style="cardContentStyle"
            size="large"
            :class="isConfigTab ? 'settings-card settings-card--config' : 'settings-card'"
          >
            <template #header>
              <div>{{ $t('settings.title') }}</div>
            </template>
            <template #header-extra>
              <n-button
                text
                :disabled="logModalOpen"
                @click="emit('close')"
                :aria-label="$t('common.close')"
                :title="$t('common.close')"
              >
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
            <n-tab-pane name="general" :tab="$t('settings.tabs.general')">
              <GeneralSettings :settings="localSettings" @update="updateSetting" />
            </n-tab-pane>

            <n-tab-pane name="shortcuts" :tab="$t('settings.tabs.shortcuts')">
              <ShortcutsSettings :settings="localSettings" @update-shortcut="updateShortcut" />
            </n-tab-pane>

            <n-tab-pane name="chat" :tab="$t('settings.tabs.chat')">
              <ChatProviderSettings
                :config="localSettings.chat"
                @update="(config) => localSettings.chat = config"
              />
            </n-tab-pane>

            <n-tab-pane name="ai" :tab="$t('settings.tabs.ai')">
              <AiInspectionSettings :settings="localSettings.ai" @update="(ai) => localSettings.ai = ai" />
            </n-tab-pane>

            <n-tab-pane name="config" :tab="$t('settings.tabs.config')">
              <ConfigCenterSettings
                :config-loading="configLoading"
                :config-busy="configBusy"
                :log-modal-open="logModalOpen"
                :selected-profile="selectedProfile"
                :profile-options="profileOptions"
                :can-delete-profile="canDeleteProfile"
                :highlight="highlightConfig"
                :proxy-config="proxyConfig"
                :broker-config="brokerConfig"
                v-model:proxy-config-text="proxyConfigText"
                v-model:broker-config-text="brokerConfigText"
                :proxy-dirty="proxyDirty"
                :broker-dirty="brokerDirty"
                :active-profile="activeProfile"
                :resolved-theme="props.resolvedTheme"
                @request-profile-change="requestProfileChange"
                @open-create-profile="openCreateProfile"
                @open-delete-profile="openDeleteProfile"
                @request-refresh="requestRefreshConfig"
                @close="emit('close')"
                @save="saveConfigFiles"
                @apply="requestApplyConfig"
              />
            </n-tab-pane>
          </n-tabs>

          <div v-if="activeTab !== 'config'" class="mt-6 flex justify-end gap-3">
            <n-button @click="emit('close')">{{ $t('common.cancel') }}</n-button>
            <n-button type="primary" @click="save">{{ $t('common.save') }}</n-button>
          </div>
          <template v-if="activeTab === 'config'" #footer>
            <div class="border-t border-border/50 bg-panel">
              <div class="flex items-center justify-between gap-3 px-3 py-3">
                <div class="min-w-0 truncate text-xs text-foreground-muted" :title="configStatusText">
                  {{ configStatusText }}
                </div>
                <div class="flex items-center gap-2">
                  <n-button
                    quaternary
                    :disabled="configBusy || logModalOpen || configLoading"
                    @click="requestRefreshConfig"
                  >
                    {{ $t('common.refresh') }}
                  </n-button>
                  <n-button :disabled="configBusy || logModalOpen" @click="emit('close')">
                    {{ $t('common.cancel') }}
                  </n-button>
                  <n-button :disabled="configBusy || logModalOpen || configLoading" @click="saveConfigFiles">
                    {{ $t('common.save') }}
                  </n-button>
                  <n-button
                    type="primary"
                    :disabled="configBusy || logModalOpen || configLoading"
                    @click="requestApplyConfig"
                  >
                    {{ $t('common.apply') }}
                  </n-button>
                </div>
              </div>
            </div>
          </template>
          </n-card>
        </div>
      </div>
    </div>
  </n-modal>

  <n-modal v-model:show="switchProfileOpen" :mask-closable="false" :close-on-esc="true">
    <n-card size="small" class="w-[22rem]" :bordered="true">
      <template #header>{{ $t('settings.profile.switchTitle') }}</template>
      <div class="text-sm text-foreground-muted">{{ $t('settings.profile.switchHint') }}</div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button @click="cancelProfileSwitch">{{ $t('common.cancel') }}</n-button>
          <n-button type="primary" :disabled="configBusy" @click="confirmProfileSwitch">
            {{ $t('settings.profile.switchConfirm') }}
          </n-button>
        </div>
      </template>
    </n-card>
  </n-modal>

  <n-modal v-model:show="createProfileOpen" :mask-closable="false" :close-on-esc="true">
    <n-card size="small" class="w-[22rem]" :bordered="true">
      <template #header>{{ $t('settings.profile.createTitle') }}</template>
      <div class="space-y-2">
        <n-input v-model:value="createProfileName" size="small" :placeholder="$t('settings.profile.createPlaceholder')" />
        <div class="text-xs text-foreground-muted">{{ $t('settings.profile.createHint') }}</div>
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button @click="createProfileOpen = false">{{ $t('common.cancel') }}</n-button>
          <n-button type="primary" :disabled="!createProfileValid || configBusy" @click="confirmCreateProfile">
            {{ $t('common.create') }}
          </n-button>
        </div>
      </template>
    </n-card>
  </n-modal>

  <n-modal v-model:show="deleteProfileOpen" :mask-closable="false" :close-on-esc="true">
    <n-card size="small" class="w-[22rem]" :bordered="true">
      <template #header>{{ $t('settings.profile.deleteTitle') }}</template>
      <div class="space-y-2">
        <n-select
          v-model:value="deleteProfileName"
          :options="deletableProfileOptions"
          size="small"
          :placeholder="$t('settings.profile.deletePlaceholder')"
        />
        <div class="text-xs text-warning">{{ $t('settings.profile.deleteHint') }}</div>
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button @click="deleteProfileOpen = false">{{ $t('common.cancel') }}</n-button>
          <n-button type="error" :disabled="!deleteProfileName || configBusy" @click="confirmDeleteProfile">
            {{ $t('settings.profile.deleteConfirm') }}
          </n-button>
        </div>
      </template>
    </n-card>
  </n-modal>

  <n-modal v-model:show="refreshConfirmOpen" :mask-closable="false" :close-on-esc="true">
    <n-card size="small" class="w-[22rem]" :bordered="true">
      <template #header>{{ $t('settings.config.refreshTitle') }}</template>
      <div class="text-sm text-foreground-muted">{{ $t('settings.config.refreshHint') }}</div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button @click="cancelRefreshConfirm">{{ $t('common.cancel') }}</n-button>
          <n-button type="primary" :disabled="configBusy" @click="confirmRefresh">
            {{ $t('settings.config.refreshConfirm') }}
          </n-button>
        </div>
      </template>
    </n-card>
  </n-modal>

  <n-modal v-model:show="logModalOpen" :mask-closable="false" :close-on-esc="false">
    <n-card size="small" class="w-[36rem]" :bordered="true">
      <template #header>{{ logTitle }}</template>
      <div class="text-sm text-foreground-muted">
        {{ logStatusMessage || $t('settings.log.preparing') }}
      </div>
      <div class="relative mt-3 h-64 overflow-hidden rounded border border-border bg-panel-muted">
        <div ref="logTerminalRef" class="h-full w-full" />
        <div
          v-if="!logHasOutput"
          class="absolute inset-0 flex items-center justify-center text-xs text-foreground-muted pointer-events-none"
        >
          {{ $t('settings.log.empty') }}
        </div>
      </div>
      <template #footer>
        <div class="flex items-center justify-between gap-3">
          <div class="flex items-center gap-2 text-xs text-foreground-muted">
            <n-spin v-if="logInProgress" size="small" />
            <span>{{ logFooterText }}</span>
          </div>
          <n-button type="primary" :disabled="logInProgress" @click="closeLogModal">
            {{ $t('common.done') }}
          </n-button>
        </div>
      </template>
    </n-card>
  </n-modal>
</template>

<style scoped>
.settings-card-shell {
  width: 100%;
  overflow-x: hidden;
  overflow-y: visible;
}

:deep(.settings-modal) {
  box-shadow: none;
  background: transparent;
}

.settings-modal-root {
  display: flex;
  justify-content: center;
}

.settings-card-shell--ready {
  transition: max-width 320ms cubic-bezier(0.22, 1, 0.36, 1),
    height 320ms cubic-bezier(0.22, 1, 0.36, 1);
  will-change: max-width, height;
}

.settings-card--config {
  display: flex;
  flex-direction: column;
}

.settings-card--config :deep(.n-card__content) {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.settings-card--config :deep(.n-card__footer) {
  padding: 0;
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
  overflow: hidden;
}

</style>
