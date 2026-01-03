<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch, type CSSProperties } from 'vue';
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
} from '../../services/api';
import { loadSettings } from '../../services/settings';
import type { AppSettings, ConfigFilePayload, ProfileSummary } from '../../shared/types';
import { type ResolvedTheme } from '../../shared/theme';
import { AiInspectionSettings, ChatProviderSettings, ConfigCenterSettings, GeneralSettings, ShortcutsSettings } from './settings';
import type { ChatProviderConfig } from '../../shared/types';

const props = defineProps<{
  isOpen: boolean;
  settings: AppSettings;
  resolvedTheme: ResolvedTheme;
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
    chat: {
      provider: source.chat.provider,
      openai: { ...source.chat.openai },
      acp: { ...source.chat.acp },
    },
    shortcuts: { ...source.shortcuts },
  };
}

const localSettings = ref<AppSettings>(cloneSettings(props.settings));
const activeTab = ref<'general' | 'shortcuts' | 'chat' | 'ai' | 'config'>('general');
const aiProviderOptions: SelectOption[] = [{ value: 'openai', label: 'OpenAI 兼容' }];
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
const notification = useNotification();
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

function updateSetting(key: keyof AppSettings, value: unknown) {
  (localSettings.value as Record<string, unknown>)[key] = value;
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
  try {
    await loadProfiles(true);
    if (selectedProfile.value) {
      await loadConfigFiles(selectedProfile.value, true);
    }
    configLoaded.value = true;
  } catch (err) {
    showConfigMessage(`加载配置失败：${String(err)}`, 'error');
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

onBeforeUnmount(() => {
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
      cardHeight.value = null;
      cardShellReady.value = false;
      void syncCardHeight();
      return;
    }
    if (!open) {
      configLoaded.value = false;
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
          <n-card :bordered="true" :style="cardStyle" :content-style="cardContentStyle" size="large">
            <template #header>
              <div>设置</div>
            </template>
            <template #header-extra>
              <n-button
                text
                :disabled="logModalOpen"
                @click="emit('close')"
                aria-label="关闭"
                title="关闭"
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
            <n-tab-pane name="general" tab="通用设置">
              <GeneralSettings :settings="localSettings" @update="updateSetting" />
            </n-tab-pane>

            <n-tab-pane name="shortcuts" tab="快捷键设置">
              <ShortcutsSettings :settings="localSettings" @update-shortcut="updateShortcut" />
            </n-tab-pane>

            <n-tab-pane name="chat" tab="聊天设置">
              <ChatProviderSettings
                :config="localSettings.chat"
                @update="(config) => localSettings.chat = config"
              />
            </n-tab-pane>

            <n-tab-pane name="ai" tab="AI 检查">
              <AiInspectionSettings :settings="localSettings.ai" @update="(ai) => localSettings.ai = ai" />
            </n-tab-pane>

            <n-tab-pane name="config" tab="配置中心">
              <ConfigCenterSettings
                :config-loading="configLoading"
                :config-busy="configBusy"
                :log-modal-open="logModalOpen"
                :selected-profile="selectedProfile"
                :profile-options="profileOptions"
                :can-delete-profile="canDeleteProfile"
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
            <n-button @click="emit('close')">取消</n-button>
            <n-button type="primary" @click="save">保存</n-button>
          </div>
          </n-card>
        </div>
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
  overflow-x: hidden;
  overflow-y: visible;
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
  line-clamp: 2;
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
