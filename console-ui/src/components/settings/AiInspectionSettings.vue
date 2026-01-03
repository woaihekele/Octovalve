<script setup lang="ts">
import { NButton, NInput, NInputNumber, NSelect, NSwitch } from 'naive-ui';
import type { SelectOption } from 'naive-ui';
import { DEFAULT_SETTINGS } from '../../settings';
import type { AppSettings } from '../../shared/types';

const props = defineProps<{
  settings: AppSettings['ai'];
}>();

const emit = defineEmits<{
  (e: 'update', settings: AppSettings['ai']): void;
}>();

const aiProviderOptions: SelectOption[] = [{ value: 'openai', label: 'OpenAI 兼容' }];

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
  updateField('prompt', DEFAULT_SETTINGS.ai.prompt);
}
</script>

<template>
  <div class="grid grid-cols-1 lg:grid-cols-[minmax(0,1fr)_minmax(0,360px)] gap-6 items-start">
    <div class="space-y-4">
      <div class="ai-field">
        <div>
          <div class="text-sm font-medium">启用 AI 检查</div>
          <div class="text-xs text-foreground-muted">对所有 Pending 命令进行风险评估</div>
        </div>
        <div class="ai-control ai-control--switch">
          <NSwitch :value="props.settings.enabled" size="small" @update:value="(v) => updateField('enabled', v)" />
        </div>
      </div>

      <div class="space-y-3">
        <div class="ai-field">
          <div>
            <div class="text-sm font-medium">Provider</div>
            <div class="text-xs text-foreground-muted">兼容 OpenAI 的接口</div>
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
            <div class="text-sm font-medium">Base URL</div>
            <div class="text-xs text-foreground-muted">模型服务地址</div>
          </div>
          <div class="ai-control">
            <NInput :value="props.settings.baseUrl" size="small" class="w-full" @update:value="(v) => updateField('baseUrl', v)" />
          </div>
        </div>

        <div class="ai-field">
          <div>
            <div class="text-sm font-medium">Chat Path</div>
            <div class="text-xs text-foreground-muted">请求路径</div>
          </div>
          <div class="ai-control">
            <NInput :value="props.settings.chatPath" size="small" class="w-full" @update:value="(v) => updateField('chatPath', v)" />
          </div>
        </div>

        <div class="ai-field">
          <div>
            <div class="text-sm font-medium">Model</div>
            <div class="text-xs text-foreground-muted">模型名称</div>
          </div>
          <div class="ai-control">
            <NInput :value="props.settings.model" size="small" class="w-full" @update:value="(v) => updateField('model', v)" />
          </div>
        </div>

        <div class="ai-field">
          <div>
            <div class="text-sm font-medium">API Key</div>
            <div class="text-xs text-foreground-muted">仅保存在本地设置</div>
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
            <div class="text-sm font-medium">Timeout (ms)</div>
            <div class="text-xs text-foreground-muted">超时后视为失败</div>
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
            <div class="text-sm font-medium">最大并发</div>
            <div class="text-xs text-foreground-muted">同时评估的请求数</div>
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
          <div class="text-sm font-medium">Prompt</div>
          <div class="text-xs text-foreground-muted" v-pre>支持 {{field}} 占位</div>
        </div>
        <NButton size="small" quaternary @click="resetPrompt">恢复默认</NButton>
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
