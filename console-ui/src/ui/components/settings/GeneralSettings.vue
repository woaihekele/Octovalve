<script setup lang="ts">
import { computed } from 'vue';
import { NSelect, NSwitch } from 'naive-ui';
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
        class="w-32"
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
  </div>
</template>
