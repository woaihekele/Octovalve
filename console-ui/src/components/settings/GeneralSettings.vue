<script setup lang="ts">
import { NSelect, NSwitch } from 'naive-ui';
import type { SelectOption } from 'naive-ui';
import type { AppSettings } from '../../types';
import { THEME_OPTIONS } from '../../theme';

const props = defineProps<{
  settings: AppSettings;
}>();

const emit = defineEmits<{
  (e: 'update', key: keyof AppSettings, value: unknown): void;
}>();

const themeOptions: SelectOption[] = THEME_OPTIONS.map((option) => ({
  value: option.value,
  label: option.label,
}));

function updateTheme(value: string) {
  emit('update', 'theme', value);
}

function updateNotifications(value: boolean) {
  emit('update', 'notificationsEnabled', value);
}
</script>

<template>
  <div class="space-y-4">
    <div class="flex items-center justify-between gap-4">
      <div>
        <div class="text-sm font-medium">主题</div>
        <div class="text-xs text-foreground-muted">系统/深色/浅色</div>
      </div>
      <NSelect
        :value="props.settings.theme"
        :options="themeOptions"
        size="small"
        class="w-32"
        @update:value="updateTheme"
      />
    </div>
    <div class="flex items-center justify-between">
      <div>
        <div class="text-sm font-medium">新 Pending 通知</div>
        <div class="text-xs text-foreground-muted">有新的待审批时弹出提示</div>
      </div>
      <NSwitch
        :value="props.settings.notificationsEnabled"
        size="small"
        @update:value="updateNotifications"
      />
    </div>
  </div>
</template>
