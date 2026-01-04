<script setup lang="ts">
import { computed } from 'vue';
import { NSelect, NInput, NInputNumber, NAlert, NSwitch } from 'naive-ui';
import type { SelectOption } from 'naive-ui';
import { useI18n } from 'vue-i18n';
import type { ChatProviderConfig } from '../../../shared/types';

const props = defineProps<{
  config: ChatProviderConfig;
}>();

const emit = defineEmits<{
  (e: 'update', config: ChatProviderConfig): void;
}>();

const { t } = useI18n();

const providerOptions = computed<SelectOption[]>(() => [
  { value: 'openai', label: t('settings.chat.provider.openai') },
  { value: 'acp', label: t('settings.chat.provider.acp') },
]);

function updateProvider(value: 'openai' | 'acp') {
  emit('update', { ...props.config, provider: value });
}

function updateSendOnEnter(value: boolean) {
  emit('update', { ...props.config, sendOnEnter: value });
}

function updateOpenaiField(field: keyof ChatProviderConfig['openai'], value: string) {
  emit('update', {
    ...props.config,
    openai: { ...props.config.openai, [field]: value },
  });
}

function updateAcpPath(value: string) {
  emit('update', {
    ...props.config,
    acp: { ...props.config.acp, path: value },
  });
}
</script>

<template>
  <div class="space-y-6">
    <!-- Provider Selection -->
    <div class="space-y-2">
      <div class="text-sm font-medium">{{ $t('settings.chat.provider.label') }}</div>
      <div class="text-xs text-foreground-muted mb-2">{{ $t('settings.chat.provider.help') }}</div>
      <NSelect
        :value="props.config.provider"
        :options="providerOptions"
        size="small"
        class="w-48"
        @update:value="updateProvider"
      />
    </div>

    <div class="flex items-center justify-between gap-4">
      <div>
        <div class="text-sm font-medium">{{ $t('settings.chat.sendShortcut') }}</div>
        <div class="text-xs text-foreground-muted">{{ $t('settings.chat.sendShortcutHelp') }}</div>
      </div>
      <div class="flex items-center gap-2">
        <span class="text-xs text-foreground-muted">{{ $t('settings.chat.sendOnEnter') }}</span>
        <NSwitch
          :value="props.config.sendOnEnter"
          size="small"
          @update:value="updateSendOnEnter"
        />
      </div>
    </div>

    <!-- OpenAI API Settings -->
    <div v-if="props.config.provider === 'openai'" class="space-y-4 p-4 rounded-lg bg-panel-muted/50">
      <div class="text-sm font-medium text-accent">{{ $t('settings.chat.openai.title') }}</div>
      
      <div class="grid gap-4">
        <div class="space-y-1">
          <div class="text-sm">{{ $t('settings.chat.openai.baseUrl') }}</div>
          <div class="text-xs text-foreground-muted">{{ $t('settings.chat.openai.baseUrlHelp') }}</div>
          <NInput
            :value="props.config.openai.baseUrl"
            size="small"
            placeholder="https://api.openai.com/v1"
            @update:value="(v) => updateOpenaiField('baseUrl', v)"
          />
        </div>

        <div class="space-y-1">
          <div class="text-sm">{{ $t('settings.chat.openai.chatPath') }}</div>
          <div class="text-xs text-foreground-muted">{{ $t('settings.chat.openai.chatPathHelp') }}</div>
          <NInput
            :value="props.config.openai.chatPath"
            size="small"
            placeholder="/chat/completions"
            @update:value="(v) => updateOpenaiField('chatPath', v)"
          />
        </div>

        <div class="space-y-1">
          <div class="text-sm">{{ $t('settings.chat.openai.model') }}</div>
          <div class="text-xs text-foreground-muted">{{ $t('settings.chat.openai.modelHelp') }}</div>
          <NInput
            :value="props.config.openai.model"
            size="small"
            placeholder="gpt-4"
            @update:value="(v) => updateOpenaiField('model', v)"
          />
        </div>

        <div class="space-y-1">
          <div class="text-sm">{{ $t('settings.chat.openai.apiKey') }}</div>
          <div class="text-xs text-foreground-muted">{{ $t('settings.chat.openai.apiKeyHelp') }}</div>
          <NInput
            :value="props.config.openai.apiKey"
            type="password"
            show-password-on="click"
            size="small"
            placeholder="sk-..."
            @update:value="(v) => updateOpenaiField('apiKey', v)"
          />
        </div>
      </div>
    </div>

    <!-- ACP Settings -->
    <div v-if="props.config.provider === 'acp'" class="space-y-4 p-4 rounded-lg bg-panel-muted/50">
      <div class="text-sm font-medium text-accent">{{ $t('settings.chat.acp.title') }}</div>
      
      <NAlert type="info" :bordered="false" class="mb-4">
        {{ $t('settings.chat.acp.hint') }}
      </NAlert>

      <div class="space-y-1">
        <div class="text-sm">{{ $t('settings.chat.acp.path') }}</div>
        <div class="text-xs text-foreground-muted">{{ $t('settings.chat.acp.pathHelp') }}</div>
        <NInput
          :value="props.config.acp.path"
          size="small"
          placeholder="/usr/local/bin/codex-acp"
          @update:value="updateAcpPath"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.bg-panel-muted\/50 {
  background: rgb(var(--color-panel-muted) / 0.5);
}
</style>
