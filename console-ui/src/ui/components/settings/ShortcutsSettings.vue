<script setup lang="ts">
import { ref } from 'vue';
import { NButton, NInput } from 'naive-ui';
import { eventToShortcut, formatShortcut, normalizeShortcut } from '../../../shared/shortcuts';
import { DEFAULT_SETTINGS } from '../../../services/settings';
import type { AppSettings } from '../../../shared/types';

const props = defineProps<{
  settings: AppSettings;
}>();

const emit = defineEmits<{
  (e: 'update-shortcut', key: string, value: string): void;
}>();

const shortcutFields = [
  { key: 'prevTarget', label: '上一个目标' },
  { key: 'nextTarget', label: '下一个目标' },
  { key: 'jumpNextPending', label: '跳转到下一个 Pending' },
  { key: 'approve', label: '批准' },
  { key: 'deny', label: '拒绝' },
  { key: 'toggleList', label: '切换 Pending/History' },
  { key: 'fullScreen', label: '全屏输出' },
  { key: 'openSettings', label: '打开设置' },
] as const;

type ShortcutField = (typeof shortcutFields)[number]['key'];
const activeShortcut = ref<ShortcutField | null>(null);

function captureShortcut(field: ShortcutField, event: KeyboardEvent) {
  if (event.code === 'Escape') {
    activeShortcut.value = null;
    return;
  }
  const shortcut = eventToShortcut(event);
  if (!shortcut) return;
  const normalized = normalizeShortcut(shortcut);
  if (!normalized) return;
  emit('update-shortcut', field, normalized);
}

function shortcutDisplay(field: ShortcutField) {
  const formatted = formatShortcut(props.settings.shortcuts[field]);
  if (activeShortcut.value === field) {
    return formatted || '按键盘设置快捷键';
  }
  return formatted || '点击设置快捷键';
}

function shortcutHasValue(field: ShortcutField) {
  return Boolean(formatShortcut(props.settings.shortcuts[field]));
}

function shortcutIsDefault(field: ShortcutField) {
  const current = normalizeShortcut(props.settings.shortcuts[field] ?? '') ?? '';
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
  emit('update-shortcut', field, '');
}

function resetShortcut(field: ShortcutField) {
  emit('update-shortcut', field, DEFAULT_SETTINGS.shortcuts[field]);
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
  <div class="space-y-3">
    <div
      v-for="item in shortcutFields"
      :key="item.key"
      class="flex items-center justify-between gap-4 text-sm"
    >
      <span class="text-foreground-muted">{{ item.label }}</span>
      <div class="flex items-center gap-2">
        <div class="w-[120px] flex-none">
          <NInput
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
        <NButton
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
        </NButton>
        <NButton
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
        </NButton>
      </div>
    </div>
  </div>
</template>
