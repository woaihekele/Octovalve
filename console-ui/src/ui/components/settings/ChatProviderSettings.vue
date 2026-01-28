<script setup lang="ts">
import { computed } from 'vue';
import { NSelect, NInput, NInputNumber, NSwitch } from 'naive-ui';
import type { SelectOption } from 'naive-ui';
import { useI18n } from 'vue-i18n';
import type { ChatProviderConfig } from '../../../shared/types';

const props = defineProps<{
  config: ChatProviderConfig;
}>();

const emit = defineEmits<{
  (e: 'update', config: ChatProviderConfig): void;
}>();

const { t, tm } = useI18n();

const mcpPlaceholder = computed(() => {
  const message = (tm as (key: string) => unknown)('settings.chat.mcp.placeholder');
  return typeof message === 'string' ? message : '';
});

const providerOptions = computed<SelectOption[]>(() => [
  { value: 'openai', label: t('settings.chat.provider.openai') },
  { value: 'acp', label: t('settings.chat.provider.acp') },
]);

const approvalPolicyOptions = computed<SelectOption[]>(() => [
  { value: 'auto', label: t('settings.chat.acp.approvalPolicy.auto') },
  { value: 'unless-trusted', label: t('settings.chat.acp.approvalPolicy.unlessTrusted') },
  { value: 'on-failure', label: t('settings.chat.acp.approvalPolicy.onFailure') },
  { value: 'on-request', label: t('settings.chat.acp.approvalPolicy.onRequest') },
  { value: 'never', label: t('settings.chat.acp.approvalPolicy.never') },
]);

const sandboxModeOptions = computed<SelectOption[]>(() => [
  { value: 'auto', label: t('settings.chat.acp.sandboxMode.auto') },
  { value: 'read-only', label: t('settings.chat.acp.sandboxMode.readOnly') },
  { value: 'workspace-write', label: t('settings.chat.acp.sandboxMode.workspaceWrite') },
  { value: 'danger-full-access', label: t('settings.chat.acp.sandboxMode.dangerFullAccess') },
]);

function updateProvider(value: 'openai' | 'acp') {
  emit('update', { ...props.config, provider: value });
}

function updateSendOnEnter(value: boolean) {
  emit('update', { ...props.config, sendOnEnter: value });
}

function updateMcpConfigJson(value: string) {
  emit('update', { ...props.config, mcpConfigJson: value });
}

function updateOpenaiField(field: keyof ChatProviderConfig['openai'], value: string) {
  emit('update', {
    ...props.config,
    openai: { ...props.config.openai, [field]: value },
  });
}

function updateAcpArgs(value: string) {
  emit('update', {
    ...props.config,
    acp: { ...props.config.acp, codexPath: value },
  });
}

function updateAcpApprovalPolicy(value: ChatProviderConfig['acp']['approvalPolicy']) {
  emit('update', {
    ...props.config,
    acp: { ...props.config.acp, approvalPolicy: value },
  });
}

function updateAcpSandboxMode(value: ChatProviderConfig['acp']['sandboxMode']) {
  emit('update', {
    ...props.config,
    acp: { ...props.config.acp, sandboxMode: value },
  });
}
</script>

<template>
  <div class="chat-settings grid min-h-0 grid-cols-1 lg:grid-cols-[minmax(0,1fr)_minmax(0,360px)] gap-6 items-stretch">
    <div class="flex flex-col gap-6 min-h-0">
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

      <!-- Provider Selection -->
      <div class="space-y-2">
        <div class="text-sm font-medium">{{ $t('settings.chat.provider.label') }}</div>
        <div class="text-xs text-foreground-muted mb-2">{{ $t('settings.chat.provider.help') }}</div>
        <NSelect
          :value="props.config.provider"
          :options="providerOptions"
          size="small"
          class="w-56"
          @update:value="updateProvider"
        />
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

        <div class="space-y-1">
          <div class="text-sm">{{ $t('settings.chat.acp.arguments') }}</div>
          <div class="text-xs text-foreground-muted">{{ $t('settings.chat.acp.argumentsHelp') }}</div>
          <NInput
            :value="props.config.acp.codexPath"
            size="small"
            placeholder="/opt/homebrew/bin/codex"
            @update:value="updateAcpArgs"
          />
        </div>

        <div class="space-y-1">
          <div class="text-sm">{{ $t('settings.chat.acp.approvalPolicy.label') }}</div>
          <div class="text-xs text-foreground-muted">{{ $t('settings.chat.acp.approvalPolicy.help') }}</div>
          <NSelect
            :value="props.config.acp.approvalPolicy"
            :options="approvalPolicyOptions"
            size="small"
            class="w-56"
            @update:value="updateAcpApprovalPolicy"
          />
        </div>

        <div class="space-y-1">
          <div class="text-sm">{{ $t('settings.chat.acp.sandboxMode.label') }}</div>
          <div class="text-xs text-foreground-muted">{{ $t('settings.chat.acp.sandboxMode.help') }}</div>
          <NSelect
            :value="props.config.acp.sandboxMode"
            :options="sandboxModeOptions"
            size="small"
            class="w-56"
            @update:value="updateAcpSandboxMode"
          />
        </div>
      </div>
    </div>

    <div class="chat-mcp-panel flex flex-col min-h-0 space-y-2">
      <div class="text-sm font-medium">{{ $t('settings.chat.mcp.title') }}</div>
      <div class="text-xs text-foreground-muted">{{ $t('settings.chat.mcp.help') }}</div>
      <div class="chat-mcp-input flex-1 min-h-0 overflow-hidden">
        <NInput
          :value="props.config.mcpConfigJson"
          type="textarea"
          size="small"
          :input-props="{ style: { resize: 'none' } }"
          :resizable="false"
          :placeholder="mcpPlaceholder"
          @update:value="updateMcpConfigJson"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.bg-panel-muted\/50 {
  background: rgb(var(--color-panel-muted) / 0.5);
}

.chat-mcp-panel :deep(.n-input__textarea textarea),
.chat-mcp-panel :deep(.n-input__textarea-el) {
  color: rgb(var(--color-text));
  resize: none !important;
  overflow: auto;
}

.chat-mcp-panel :deep(.n-input__textarea),
.chat-mcp-panel :deep(.n-input__textarea-el) {
  resize: none !important;
}

.chat-mcp-panel :deep(.n-input__textarea textarea::-webkit-resizer),
.chat-mcp-panel :deep(.n-input__textarea-el::-webkit-resizer) {
  display: none;
}

.chat-mcp-panel :deep(.n-input__textarea textarea::placeholder) {
  color: rgb(var(--color-text-muted));
  opacity: 0.4;
}

.chat-mcp-panel :deep(.n-input__placeholder) {
  color: rgb(var(--color-text-muted));
  opacity: 0.4;
}

.chat-mcp-input :deep(.n-input),
.chat-mcp-input :deep(.n-input__textarea),
.chat-mcp-input :deep(.n-input__textarea textarea) {
  height: 100%;
}

.chat-settings :deep(.n-input:not(.n-input--disabled) .n-input__input input),
.chat-settings :deep(.n-input:not(.n-input--disabled) .n-input__textarea textarea),
.chat-settings :deep(.n-input-number:not(.n-input-number--disabled) .n-input__input input),
.chat-settings :deep(.n-base-selection:not(.n-base-selection--disabled) .n-base-selection-label) {
  color: rgb(var(--color-text));
}

.chat-settings :deep(.n-input:not(.n-input--disabled) .n-input__input input::placeholder),
.chat-settings :deep(.n-input:not(.n-input--disabled) .n-input__textarea textarea::placeholder),
.chat-settings :deep(.n-input-number:not(.n-input-number--disabled) .n-input__input input::placeholder),
.chat-settings :deep(.n-base-selection:not(.n-base-selection--disabled) .n-base-selection-placeholder) {
  color: rgb(var(--color-text-muted));
}
</style>
