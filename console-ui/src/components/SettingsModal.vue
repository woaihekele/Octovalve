<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
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
    shortcuts: { ...source.shortcuts },
  };
}

const localSettings = ref<AppSettings>(cloneSettings(props.settings));
const activeTab = ref<'general' | 'shortcuts' | 'config'>('general');
const themeOptions = [
  { value: 'system', label: '系统' },
  { value: 'dark', label: '深色' },
  { value: 'light', label: '浅色' },
] as const;
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

function save() {
  emit('save', localSettings.value);
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

function handleEscape(event: KeyboardEvent) {
  if (!hasOpen.value) {
    return;
  }
  if (event.key === 'Escape') {
    event.preventDefault();
    emit('close');
  }
}

onMounted(() => {
  window.addEventListener('keydown', handleEscape);
});

onBeforeUnmount(() => {
  window.removeEventListener('keydown', handleEscape);
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
    }
  }
);
</script>

<template>
  <div v-if="hasOpen" class="fixed inset-0 z-50 flex items-center justify-center">
    <div class="absolute inset-0 bg-black/60"></div>
    <div
      class="relative w-full bg-panel border border-border rounded-xl shadow-xl p-6 flex flex-col"
      :class="isConfigTab ? 'max-w-5xl h-[80vh]' : 'max-w-lg'"
    >
      <div class="flex items-center justify-between">
        <h2 class="text-lg font-semibold">设置</h2>
        <button
          class="text-foreground-muted hover:text-foreground p-1 rounded hover:bg-panel-muted transition-colors"
          @click="emit('close')"
          aria-label="关闭"
          title="关闭"
        >
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>

      <div class="mt-4 flex items-center gap-2 border-b border-border pb-2 text-sm">
        <button
          class="px-3 py-1.5 rounded border transition-colors"
          :class="activeTab === 'general' ? 'border-accent text-accent bg-accent/10' : 'border-border text-foreground-muted hover:text-foreground'"
          @click="activeTab = 'general'"
        >
          通用设置
        </button>
        <button
          class="px-3 py-1.5 rounded border transition-colors"
          :class="activeTab === 'shortcuts' ? 'border-accent text-accent bg-accent/10' : 'border-border text-foreground-muted hover:text-foreground'"
          @click="activeTab = 'shortcuts'"
        >
          快捷键设置
        </button>
        <button
          class="px-3 py-1.5 rounded border transition-colors"
          :class="activeTab === 'config' ? 'border-accent text-accent bg-accent/10' : 'border-border text-foreground-muted hover:text-foreground'"
          @click="activeTab = 'config'"
        >
          配置中心
        </button>
      </div>

      <div class="mt-4 flex-1 min-h-0">
        <div v-if="activeTab === 'general'" class="space-y-4">
          <div class="flex items-center justify-between gap-4">
            <div>
              <div class="text-sm font-medium">主题</div>
              <div class="text-xs text-foreground-muted">系统/深色/浅色</div>
            </div>
            <select
              v-model="localSettings.theme"
              class="bg-panel-muted border border-border rounded px-2 py-1 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-accent/40"
            >
              <option v-for="option in themeOptions" :key="option.value" :value="option.value">
                {{ option.label }}
              </option>
            </select>
          </div>
          <div class="flex items-center justify-between">
            <div>
              <div class="text-sm font-medium">新 Pending 通知</div>
              <div class="text-xs text-foreground-muted">有新的待审批时弹出提示</div>
            </div>
            <input
              type="checkbox"
              v-model="localSettings.notificationsEnabled"
              class="h-4 w-4 accent-accent"
            />
          </div>
        </div>

        <div v-else-if="activeTab === 'shortcuts'" class="space-y-3">
          <div
            v-for="item in shortcutFields"
            :key="item.key"
            class="flex items-center justify-between gap-4 text-sm"
          >
            <span class="text-foreground-muted">{{ item.label }}</span>
            <div class="flex items-center gap-2">
              <input
                :value="shortcutDisplay(item.key)"
                class="w-44 bg-panel-muted border rounded px-2 py-1 text-sm transition-colors cursor-pointer"
                :class="shortcutInputClass(item.key)"
                readonly
                @focus="activateShortcut(item.key)"
                @blur="deactivateShortcut(item.key)"
                @keydown.prevent="captureShortcut(item.key, $event)"
              />
              <button
                type="button"
                class="h-8 w-8 flex items-center justify-center rounded border border-border text-foreground-muted hover:text-foreground hover:border-foreground-muted transition-colors"
                :class="shortcutHasValue(item.key) ? '' : 'opacity-40 cursor-not-allowed'"
                :disabled="!shortcutHasValue(item.key)"
                title="清空"
                aria-label="清空快捷键"
                @click="clearShortcut(item.key)"
              >
                <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                  <line x1="18" y1="6" x2="6" y2="18" />
                  <line x1="6" y1="6" x2="18" y2="18" />
                </svg>
              </button>
              <button
                type="button"
                class="h-8 w-8 flex items-center justify-center rounded border border-border text-foreground-muted hover:text-foreground hover:border-foreground-muted transition-colors"
                :class="shortcutIsDefault(item.key) ? 'opacity-40 cursor-not-allowed' : ''"
                :disabled="shortcutIsDefault(item.key)"
                title="恢复默认"
                aria-label="恢复默认快捷键"
                @click="resetShortcut(item.key)"
              >
                <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M3 12a9 9 0 1 0 3-6.7" />
                  <polyline points="3 4 3 10 9 10" />
                </svg>
              </button>
            </div>
          </div>
        </div>

        <div v-else class="flex flex-col gap-4 min-h-0">
          <div v-if="configLoading" class="text-sm text-foreground-muted">正在加载配置...</div>
          <div v-else-if="configError" class="text-sm text-danger border border-danger/30 bg-danger/10 rounded px-3 py-2">
            加载失败：{{ configError }}
          </div>
          <div v-else class="grid grid-cols-1 lg:grid-cols-2 gap-4 min-h-0">
            <div class="flex flex-col gap-2 min-h-0">
              <div class="flex items-center justify-between text-sm">
                <div>
                  <div class="font-medium">本地代理配置</div>
                  <div class="text-xs text-foreground-muted break-all">{{ proxyConfig?.path }}</div>
                </div>
                <span v-if="proxyConfig && !proxyConfig.exists" class="text-xs text-warning">未创建</span>
              </div>
              <MonacoEditor v-model="proxyConfigText" language="toml" height="280px" :theme="props.resolvedTheme" />
              <div class="text-xs text-foreground-muted">修改 broker_config_path 后建议点击刷新或保存并应用。</div>
            </div>

            <div class="flex flex-col gap-2 min-h-0">
              <div class="flex items-center justify-between text-sm">
                <div>
                  <div class="font-medium">远端 broker 配置（源文件）</div>
                  <div class="text-xs text-foreground-muted break-all">{{ brokerConfig?.path }}</div>
                </div>
              </div>
              <MonacoEditor v-model="brokerConfigText" language="toml" height="280px" :theme="props.resolvedTheme" />
            </div>
          </div>

          <div v-if="configMessage" class="text-xs text-success border border-success/20 bg-success/10 rounded px-3 py-2">
            {{ configMessage }}
          </div>
        </div>
      </div>

      <div v-if="activeTab !== 'config'" class="mt-6 flex justify-end gap-3">
        <button class="px-4 py-2 rounded bg-panel-muted text-foreground" @click="emit('close')">取消</button>
        <button class="px-4 py-2 rounded bg-accent text-white" @click="save">保存</button>
      </div>

      <div v-else class="mt-4 flex items-center justify-between gap-3">
        <div class="text-xs text-foreground-muted">
          本地配置 {{ proxyDirty ? '有改动' : '未改动' }} · 远端配置 {{ brokerDirty ? '有改动' : '未改动' }}
        </div>
        <div class="flex items-center gap-2">
          <button
            class="px-3 py-2 rounded border border-border text-foreground hover:border-foreground-muted disabled:opacity-40"
            :disabled="configBusy"
            @click="loadConfigCenter"
          >
            刷新
          </button>
          <button
            class="px-3 py-2 rounded border border-border text-foreground hover:border-foreground-muted disabled:opacity-40"
            :disabled="configBusy || !proxyDirty"
            @click="saveProxyConfig"
          >
            保存本地
          </button>
          <button
            class="px-3 py-2 rounded border border-border text-foreground hover:border-foreground-muted disabled:opacity-40"
            :disabled="configBusy || !brokerDirty"
            @click="saveBrokerConfig"
          >
            保存远端
          </button>
          <button
            class="px-3 py-2 rounded bg-accent text-white disabled:opacity-40"
            :disabled="configBusy"
            @click="saveAndApply"
          >
            保存并应用
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
