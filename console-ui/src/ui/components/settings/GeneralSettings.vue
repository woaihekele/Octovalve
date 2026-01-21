<script setup lang="ts">
import { computed } from 'vue';
import { NInputNumber, NSelect, NSwitch } from 'naive-ui';
import type { SelectOption } from 'naive-ui';
import { useI18n } from 'vue-i18n';
import type { AppSettings } from '../../../shared/types';
import { THEME_OPTIONS } from '../../../shared/theme';

const props = defineProps<{
  settings: AppSettings;
}>();

const emit = defineEmits<{
  (e: 'update', key: keyof AppSettings, value: unknown): void;
}>();

const { t } = useI18n();

const themeOptions = computed<SelectOption[]>(() =>
  THEME_OPTIONS.map((option) => ({
    value: option.value,
    label: t(option.labelKey),
  }))
);

const languageOptions = computed<SelectOption[]>(() => [
  { value: 'zh-CN', label: t('language.zh') },
  { value: 'en-US', label: t('language.en') },
]);

function updateTheme(value: string) {
  emit('update', 'theme', value);
}

function updateNotifications(value: boolean) {
  emit('update', 'notificationsEnabled', value);
}

function updateLanguage(value: string) {
  emit('update', 'language', value);
}

function updateUiScale(value: number | null) {
  emit('update', 'uiScale', value ?? 1);
}

function updateTerminalScale(value: number | null) {
  emit('update', 'terminalScale', value ?? 1);
}
</script>

<template>
  <div class="space-y-4">
    <div class="flex items-center justify-between gap-4">
      <div>
        <div class="text-sm font-medium">{{ $t('settings.general.theme') }}</div>
        <div class="text-xs text-foreground-muted">{{ $t('settings.general.themeHelp') }}</div>
      </div>
      <NSelect
        :value="props.settings.theme"
        :options="themeOptions"
        size="small"
        class="w-44"
        to="body"
        @update:value="updateTheme"
      />
    </div>
    <div class="flex items-center justify-between gap-4">
      <div>
        <div class="text-sm font-medium">{{ $t('settings.general.language') }}</div>
        <div class="text-xs text-foreground-muted">{{ $t('settings.general.languageHelp') }}</div>
      </div>
      <NSelect
        :value="props.settings.language"
        :options="languageOptions"
        size="small"
        class="w-32"
        to="body"
        @update:value="updateLanguage"
      />
    </div>
    <div class="flex items-center justify-between">
      <div>
        <div class="text-sm font-medium">{{ $t('settings.general.notifications') }}</div>
        <div class="text-xs text-foreground-muted">{{ $t('settings.general.notificationsHelp') }}</div>
      </div>
      <NSwitch
        :value="props.settings.notificationsEnabled"
        size="small"
        @update:value="updateNotifications"
      />
    </div>
    <div class="flex items-center justify-between gap-4">
      <div>
        <div class="text-sm font-medium">{{ $t('settings.general.uiScale') }}</div>
        <div class="text-xs text-foreground-muted">{{ $t('settings.general.uiScaleHelp') }}</div>
      </div>
      <NInputNumber
        :value="props.settings.uiScale"
        size="small"
        :min="0.8"
        :max="1.5"
        :step="0.1"
        :precision="1"
        class="w-28"
        @update:value="updateUiScale"
      />
    </div>
    <div class="flex items-center justify-between gap-4">
      <div>
        <div class="text-sm font-medium">{{ $t('settings.general.terminalScale') }}</div>
        <div class="text-xs text-foreground-muted">{{ $t('settings.general.terminalScaleHelp') }}</div>
      </div>
      <NInputNumber
        :value="props.settings.terminalScale"
        size="small"
        :min="0.8"
        :max="1.5"
        :step="0.1"
        :precision="1"
        class="w-28"
        @update:value="updateTerminalScale"
      />
    </div>
  </div>
</template>
