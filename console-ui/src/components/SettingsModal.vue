<script setup lang="ts">
import { computed, ref, watch } from 'vue';
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
            <label class="flex items-center justify-between gap-4 text-sm">
              <span class="text-slate-400">跳转到下一个 Pending（Alt + 键）</span>
              <input
                v-model="localSettings.shortcuts.jumpNextPending"
                class="w-28 bg-slate-800 border border-slate-700 rounded px-2 py-1 text-slate-200"
              />
            </label>
            <label class="flex items-center justify-between gap-4 text-sm">
              <span class="text-slate-400">批准</span>
              <input
                v-model="localSettings.shortcuts.approve"
                class="w-28 bg-slate-800 border border-slate-700 rounded px-2 py-1 text-slate-200"
              />
            </label>
            <label class="flex items-center justify-between gap-4 text-sm">
              <span class="text-slate-400">拒绝</span>
              <input
                v-model="localSettings.shortcuts.deny"
                class="w-28 bg-slate-800 border border-slate-700 rounded px-2 py-1 text-slate-200"
              />
            </label>
            <label class="flex items-center justify-between gap-4 text-sm">
              <span class="text-slate-400">切换 Pending/History</span>
              <input
                v-model="localSettings.shortcuts.toggleList"
                class="w-28 bg-slate-800 border border-slate-700 rounded px-2 py-1 text-slate-200"
              />
            </label>
            <label class="flex items-center justify-between gap-4 text-sm">
              <span class="text-slate-400">全屏输出</span>
              <input
                v-model="localSettings.shortcuts.fullScreen"
                class="w-28 bg-slate-800 border border-slate-700 rounded px-2 py-1 text-slate-200"
              />
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
