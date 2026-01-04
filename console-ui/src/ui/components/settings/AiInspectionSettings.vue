<script setup lang="ts">
import { computed } from 'vue';
import { NButton, NInput, NInputNumber, NSelect, NSwitch } from 'naive-ui';
import type { SelectOption } from 'naive-ui';
import { useI18n } from 'vue-i18n';
import { DEFAULT_SETTINGS, getDefaultAiPrompt } from '../../../services/settings';
import type { AppLanguage, AppSettings } from '../../../shared/types';

const props = defineProps<{
  settings: AppSettings['ai'];
}>();

const emit = defineEmits<{
  (e: 'update', settings: AppSettings['ai']): void;
}>();

const { t, locale } = useI18n();

const aiProviderOptions = computed<SelectOption[]>(() => [
  { value: 'openai', label: t('settings.ai.provider.openai') },
]);

function updateField<K extends keyof AppSettings['ai']>(field: K, value: AppSettings['ai'][K]) {
  emit('update', { ...props.settings, [field]: value });
}

function updateTimeout(value: number | null) {
  updateField('timeoutMs', value ?? DEFAULT_SETTINGS.ai.timeoutMs);
}

function updateMaxConcurrency(value: number | null) {
  updateField('maxConcurrency', value ?? DEFAULT_SETTINGS.ai.maxConcurrency);
}

function resetPrompt() {
  updateField('prompt', getDefaultAiPrompt(locale.value as AppLanguage));
}
</script>

<template>
  <div class="grid grid-cols-1 lg:grid-cols-[minmax(0,1fr)_minmax(0,360px)] gap-6 items-start">
    <div class="space-y-4">
      <div class="ai-field">
        <div>
          <div class="text-sm font-medium">{{ $t('settings.ai.enable') }}</div>
          <div class="text-xs text-foreground-muted">{{ $t('settings.ai.enableHelp') }}</div>
        </div>
        <div class="ai-control ai-control--switch">
          <NSwitch :value="props.settings.enabled" size="small" @update:value="(v) => updateField('enabled', v)" />
        </div>
      </div>

      <div class="ai-field">
        <div>
          <div class="text-sm font-medium">{{ $t('settings.ai.autoApproveLowRisk') }}</div>
          <div class="text-xs text-foreground-muted">{{ $t('settings.ai.autoApproveLowRiskHelp') }}</div>
        </div>
        <div class="ai-control ai-control--switch">
          <NSwitch
            :value="props.settings.autoApproveLowRisk"
            size="small"
            :disabled="!props.settings.enabled"
            @update:value="(v) => updateField('autoApproveLowRisk', v)"
          />
        </div>
      </div>

      <div class="space-y-3">
        <div class="ai-field">
          <div>
            <div class="text-sm font-medium">{{ $t('settings.ai.provider.label') }}</div>
            <div class="text-xs text-foreground-muted">{{ $t('settings.ai.provider.help') }}</div>
          </div>
          <div class="ai-control">
            <NSelect
              :value="props.settings.provider"
              :options="aiProviderOptions"
              size="small"
              class="w-full"
              @update:value="(v) => updateField('provider', v as AppSettings['ai']['provider'])"
            />
          </div>
        </div>

        <div class="ai-field">
          <div>
            <div class="text-sm font-medium">{{ $t('settings.ai.baseUrl') }}</div>
            <div class="text-xs text-foreground-muted">{{ $t('settings.ai.baseUrlHelp') }}</div>
          </div>
          <div class="ai-control">
            <NInput :value="props.settings.baseUrl" size="small" class="w-full" @update:value="(v) => updateField('baseUrl', v)" />
          </div>
        </div>

        <div class="ai-field">
          <div>
            <div class="text-sm font-medium">{{ $t('settings.ai.chatPath') }}</div>
            <div class="text-xs text-foreground-muted">{{ $t('settings.ai.chatPathHelp') }}</div>
          </div>
          <div class="ai-control">
            <NInput :value="props.settings.chatPath" size="small" class="w-full" @update:value="(v) => updateField('chatPath', v)" />
          </div>
        </div>

        <div class="ai-field">
          <div>
            <div class="text-sm font-medium">{{ $t('settings.ai.model') }}</div>
            <div class="text-xs text-foreground-muted">{{ $t('settings.ai.modelHelp') }}</div>
          </div>
          <div class="ai-control">
            <NInput :value="props.settings.model" size="small" class="w-full" @update:value="(v) => updateField('model', v)" />
          </div>
        </div>

        <div class="ai-field">
          <div>
            <div class="text-sm font-medium">{{ $t('settings.ai.apiKey') }}</div>
            <div class="text-xs text-foreground-muted">{{ $t('settings.ai.apiKeyHelp') }}</div>
          </div>
          <div class="ai-control">
            <NInput
              :value="props.settings.apiKey"
              size="small"
              class="w-full"
              type="password"
              @update:value="(v) => updateField('apiKey', v)"
            />
          </div>
        </div>

        <div class="ai-field">
          <div>
            <div class="text-sm font-medium">{{ $t('settings.ai.timeout') }}</div>
            <div class="text-xs text-foreground-muted">{{ $t('settings.ai.timeoutHelp') }}</div>
          </div>
          <div class="ai-control">
            <NInputNumber
              :value="props.settings.timeoutMs"
              size="small"
              class="w-full"
              :min="1000"
              :max="60000"
              @update:value="updateTimeout"
            />
          </div>
        </div>

        <div class="ai-field">
          <div>
          <div class="text-sm font-medium">{{ $t('settings.ai.maxConcurrency') }}</div>
          <div class="text-xs text-foreground-muted">{{ $t('settings.ai.maxConcurrencyHelp') }}</div>
          </div>
          <div class="ai-control">
            <NInputNumber
              :value="props.settings.maxConcurrency"
              size="small"
              class="w-full"
              :min="1"
              :max="10"
              @update:value="updateMaxConcurrency"
            />
          </div>
        </div>
      </div>
    </div>

    <div class="flex flex-col gap-2">
      <div class="flex items-start justify-between gap-3">
        <div>
          <div class="text-sm font-medium">{{ $t('settings.ai.prompt') }}</div>
          <div class="text-xs text-foreground-muted">{{ $t('settings.ai.promptHelp') }}</div>
        </div>
        <NButton size="small" quaternary @click="resetPrompt">{{ $t('settings.ai.promptReset') }}</NButton>
      </div>
      <div class="ai-control ai-control--prompt">
        <NInput
          :value="props.settings.prompt"
          type="textarea"
          class="w-full"
          :autosize="{ minRows: 16, maxRows: 16 }"
          @update:value="(v) => updateField('prompt', v)"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
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
