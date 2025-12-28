<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { eventToShortcut, formatShortcut, normalizeShortcut } from '../shortcuts';
import { DEFAULT_SETTINGS } from '../settings';
import type { AppSettings } from '../types';

const props = defineProps<{
  isOpen: boolean;
  settings: AppSettings;
}>();

const emit = defineEmits<{
  (e: 'close'): void;
  (e: 'save', settings: AppSettings): void;
}>();

function cloneSettings(source: AppSettings): AppSettings {
  return {
    notificationsEnabled: source.notificationsEnabled,
    shortcuts: { ...source.shortcuts },
  };
}

const localSettings = ref<AppSettings>(cloneSettings(props.settings));
const shortcutFields = [
  { key: 'jumpNextPending', label: '跳转到下一个 Pending' },
  { key: 'approve', label: '批准' },
  { key: 'deny', label: '拒绝' },
  { key: 'toggleList', label: '切换 Pending/History' },
  { key: 'fullScreen', label: '全屏输出' },
] as const;
type ShortcutField = (typeof shortcutFields)[number]['key'];
const activeShortcut = ref<ShortcutField | null>(null);

watch(
  () => props.settings,
  (value) => {
    localSettings.value = cloneSettings(value);
  },
  { deep: true }
);

const hasOpen = computed(() => props.isOpen);

function save() {
  emit('save', localSettings.value);
}

function captureShortcut(field: ShortcutField, event: KeyboardEvent) {
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
    return 'border-indigo-400 text-indigo-200 ring-1 ring-indigo-400/60';
  }
  if (shortcutHasValue(field)) {
    return 'border-slate-700 text-slate-200';
  }
  return 'border-slate-700 text-slate-500';
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
</script>

<template>
  <div v-if="hasOpen" class="fixed inset-0 z-50 flex items-center justify-center">
    <div class="absolute inset-0 bg-black/60" @click="emit('close')"></div>
    <div class="relative w-full max-w-lg bg-slate-900 border border-slate-700 rounded-xl shadow-xl p-6">
      <div class="flex items-center justify-between mb-6">
        <h2 class="text-lg font-semibold">设置</h2>
        <button class="text-slate-400 hover:text-slate-200" @click="emit('close')">关闭</button>
      </div>

      <div class="space-y-4">
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm font-medium">新 Pending 通知</div>
            <div class="text-xs text-slate-500">有新的待审批时弹出提示</div>
          </div>
          <input
            type="checkbox"
            v-model="localSettings.notificationsEnabled"
            class="h-4 w-4 accent-indigo-500"
          />
        </div>

        <div class="border-t border-slate-800 pt-4">
          <div class="text-sm font-medium mb-3">快捷键</div>
          <div class="space-y-3">
            <label
              v-for="item in shortcutFields"
              :key="item.key"
              class="flex items-center justify-between gap-4 text-sm"
            >
              <span class="text-slate-400">{{ item.label }}</span>
              <div class="flex items-center gap-2">
                <input
                  :value="shortcutDisplay(item.key)"
                  class="w-44 bg-slate-800 border rounded px-2 py-1 text-sm transition-colors cursor-pointer"
                  :class="shortcutInputClass(item.key)"
                  readonly
                  @focus="activateShortcut(item.key)"
                  @blur="deactivateShortcut(item.key)"
                  @keydown.prevent="captureShortcut(item.key, $event)"
                />
                <button
                  type="button"
                  class="h-8 w-8 flex items-center justify-center rounded border border-slate-700 text-slate-400 hover:text-slate-200 hover:border-slate-500 transition-colors"
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
                  class="h-8 w-8 flex items-center justify-center rounded border border-slate-700 text-slate-400 hover:text-slate-200 hover:border-slate-500 transition-colors"
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
            </label>
          </div>
        </div>
      </div>

      <div class="mt-6 flex justify-end gap-3">
        <button class="px-4 py-2 rounded bg-slate-800 text-slate-200" @click="emit('close')">取消</button>
        <button class="px-4 py-2 rounded bg-indigo-500 text-white" @click="save">保存</button>
      </div>
    </div>
  </div>
</template>
