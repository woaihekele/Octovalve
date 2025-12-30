<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch, type CSSProperties } from 'vue';
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
import { readBrokerConfig, readProxyConfig, restartConsole, writeBrokerConfig, writeProxyConfig } from '../api';
import { eventToShortcut, formatShortcut, normalizeShortcut } from '../shortcuts';
import { DEFAULT_SETTINGS } from '../settings';
import type { AppSettings, ConfigFilePayload } from '../types';
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
const proxyConfig = ref<ConfigFilePayload | null>(null);
const brokerConfig = ref<ConfigFilePayload | null>(null);
const proxyConfigText = ref('');
const brokerConfigText = ref('');
const proxyOriginal = ref('');
const brokerOriginal = ref('');
const configLoaded = ref(false);
let configMessageTimer: number | null = null;

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
const cardStyle = computed(() => ({
  width: '100%',
  maxWidth: isConfigTab.value ? '80rem' : '32rem',
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

function save() {
  emit('save', cloneSettings(localSettings.value));
}

function showConfigMessage(message: string) {
  configMessage.value = message;
  if (configMessageTimer !== null) {
    window.clearTimeout(configMessageTimer);
  }
  configMessageTimer = window.setTimeout(() => {
    configMessage.value = null;
    configMessageTimer = null;
  }, 4000);
}

async function loadConfigCenter() {
  if (configLoading.value) {
    return;
  }
  configLoading.value = true;
  configError.value = null;
  try {
    const [proxy, broker] = await Promise.all([readProxyConfig(), readBrokerConfig()]);
    proxyConfig.value = proxy;
    brokerConfig.value = broker;
    proxyConfigText.value = proxy.content;
    brokerConfigText.value = broker.content;
    proxyOriginal.value = proxy.content;
    brokerOriginal.value = broker.content;
    configLoaded.value = true;
  } catch (err) {
    configError.value = String(err);
  } finally {
    configLoading.value = false;
  }
}

async function saveProxyConfig() {
  if (configBusy.value) {
    return;
  }
  configBusy.value = true;
  try {
    await writeProxyConfig(proxyConfigText.value);
    proxyOriginal.value = proxyConfigText.value;
    if (!brokerDirty.value) {
      await loadConfigCenter();
    } else {
      showConfigMessage('本地配置已保存，远端配置有未保存改动，未自动刷新。');
    }
  } catch (err) {
    showConfigMessage(`保存本地配置失败：${String(err)}`);
  } finally {
    configBusy.value = false;
  }
}

async function saveBrokerConfig() {
  if (configBusy.value) {
    return;
  }
  configBusy.value = true;
  try {
    await writeBrokerConfig(brokerConfigText.value);
    brokerOriginal.value = brokerConfigText.value;
    showConfigMessage('远端配置已保存。');
  } catch (err) {
    showConfigMessage(`保存远端配置失败：${String(err)}`);
  } finally {
    configBusy.value = false;
  }
}

async function saveAndApply() {
  if (configBusy.value) {
    return;
  }
  configBusy.value = true;
  try {
    await writeProxyConfig(proxyConfigText.value);
    await writeBrokerConfig(brokerConfigText.value);
    proxyOriginal.value = proxyConfigText.value;
    brokerOriginal.value = brokerConfigText.value;
    await restartConsole();
    showConfigMessage('已保存并重启 console，同步全部目标。');
  } catch (err) {
    showConfigMessage(`保存并应用失败：${String(err)}`);
  } finally {
    configBusy.value = false;
  }
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
});

watch(
  () => activeTab.value,
  (tab) => {
    if (tab === 'config' && hasOpen.value && !configLoaded.value) {
      void loadConfigCenter();
    }
  }
);

watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      localSettings.value = cloneSettings(props.settings);
      activeShortcut.value = null;
      return;
    }
    if (!open) {
      configLoaded.value = false;
      configError.value = null;
      configMessage.value = null;
      proxyConfig.value = null;
      brokerConfig.value = null;
      proxyConfigText.value = '';
      brokerConfigText.value = '';
      proxyOriginal.value = '';
      brokerOriginal.value = '';
      activeTab.value = 'general';
      activeShortcut.value = null;
    }
  }
);
</script>

<template>
  <n-modal
    :show="hasOpen"
    :mask-closable="false"
    :close-on-esc="true"
    @update:show="(value) => { if (!value) emit('close'); }"
  >
    <n-card :bordered="true" :style="cardStyle" :content-style="cardContentStyle" size="large">
      <template #header>设置</template>
      <template #header-extra>
        <n-button text @click="emit('close')" aria-label="关闭" title="关闭">
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
          <div class="space-y-4">
            <div class="flex items-center justify-between gap-4">
              <div>
                <div class="text-sm font-medium">启用 AI 检查</div>
                <div class="text-xs text-foreground-muted">对所有 Pending 命令进行风险评估</div>
              </div>
              <n-switch v-model:value="localSettings.ai.enabled" size="small" />
            </div>

            <div class="grid grid-cols-1 gap-3">
              <div class="flex items-center justify-between gap-4">
                <div>
                  <div class="text-sm font-medium">Provider</div>
                  <div class="text-xs text-foreground-muted">兼容 OpenAI 的接口</div>
                </div>
                <n-select v-model:value="localSettings.ai.provider" :options="aiProviderOptions" size="small" class="w-36" />
              </div>

              <div class="flex items-center justify-between gap-4">
                <div>
                  <div class="text-sm font-medium">Base URL</div>
                  <div class="text-xs text-foreground-muted">模型服务地址</div>
                </div>
                <n-input v-model:value="localSettings.ai.baseUrl" size="small" class="w-72" />
              </div>

              <div class="flex items-center justify-between gap-4">
                <div>
                  <div class="text-sm font-medium">Chat Path</div>
                  <div class="text-xs text-foreground-muted">请求路径</div>
                </div>
                <n-input v-model:value="localSettings.ai.chatPath" size="small" class="w-72" />
              </div>

              <div class="flex items-center justify-between gap-4">
                <div>
                  <div class="text-sm font-medium">Model</div>
                  <div class="text-xs text-foreground-muted">模型名称</div>
                </div>
                <n-input v-model:value="localSettings.ai.model" size="small" class="w-48" />
              </div>

              <div class="flex items-center justify-between gap-4">
                <div>
                  <div class="text-sm font-medium">API Key</div>
                  <div class="text-xs text-foreground-muted">仅保存在本地设置</div>
                </div>
                <n-input v-model:value="localSettings.ai.apiKey" size="small" class="w-72" type="password" />
              </div>

              <div class="flex items-center justify-between gap-4">
                <div>
                  <div class="text-sm font-medium">Timeout (ms)</div>
                  <div class="text-xs text-foreground-muted">超时后视为失败</div>
                </div>
                <n-input-number
                  :value="localSettings.ai.timeoutMs"
                  size="small"
                  class="w-40"
                  :min="1000"
                  :max="60000"
                  @update:value="(value) => { localSettings.ai.timeoutMs = value ?? DEFAULT_SETTINGS.ai.timeoutMs; }"
                />
              </div>

              <div class="flex items-center justify-between gap-4">
                <div>
                  <div class="text-sm font-medium">最大并发</div>
                  <div class="text-xs text-foreground-muted">同时评估的请求数</div>
                </div>
                <n-input-number
                  :value="localSettings.ai.maxConcurrency"
                  size="small"
                  class="w-32"
                  :min="1"
                  :max="10"
                  @update:value="(value) => { localSettings.ai.maxConcurrency = value ?? DEFAULT_SETTINGS.ai.maxConcurrency; }"
                />
              </div>
            </div>

            <div class="space-y-2">
              <div class="flex items-center justify-between">
                <div>
                  <div class="text-sm font-medium">Prompt</div>
                  <div class="text-xs text-foreground-muted" v-pre>支持 {{field}} 占位</div>
                </div>
                <n-button size="small" quaternary @click="localSettings.ai.prompt = DEFAULT_SETTINGS.ai.prompt">
                  恢复默认
                </n-button>
              </div>
              <n-input v-model:value="localSettings.ai.prompt" type="textarea" :autosize="{ minRows: 8, maxRows: 14 }" />
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
            <div v-else class="grid grid-cols-1 lg:grid-cols-2 gap-4 min-h-0 flex-1">
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

            <n-alert v-if="configMessage" type="success" :bordered="false">
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
          本地配置 {{ proxyDirty ? '有改动' : '未改动' }} · 远端配置 {{ brokerDirty ? '有改动' : '未改动' }}
        </div>
        <div class="flex items-center gap-2">
          <n-button :disabled="configBusy" @click="loadConfigCenter">刷新</n-button>
          <n-button :disabled="configBusy || !proxyDirty" @click="saveProxyConfig">保存本地</n-button>
          <n-button :disabled="configBusy || !brokerDirty" @click="saveBrokerConfig">保存远端</n-button>
          <n-button type="primary" :disabled="configBusy" @click="saveAndApply">保存并应用</n-button>
        </div>
      </div>
    </n-card>
  </n-modal>
</template>

<style scoped>
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
</style>
