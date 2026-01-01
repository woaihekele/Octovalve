<script setup lang="ts">
import { NSelect, NInput, NInputNumber, NAlert } from 'naive-ui';
import type { SelectOption } from 'naive-ui';

export interface ChatProviderConfig {
  provider: 'openai' | 'acp';
  openai: {
    baseUrl: string;
    apiKey: string;
    model: string;
    chatPath: string;
  };
  acp: {
    path: string;
  };
}

const props = defineProps<{
  config: ChatProviderConfig;
}>();

const emit = defineEmits<{
  (e: 'update', config: ChatProviderConfig): void;
}>();

const providerOptions: SelectOption[] = [
  { value: 'openai', label: 'OpenAI 兼容 API' },
  { value: 'acp', label: 'ACP (codex-acp)' },
];

function updateProvider(value: 'openai' | 'acp') {
  emit('update', { ...props.config, provider: value });
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
      <div class="text-sm font-medium">聊天 Provider</div>
      <div class="text-xs text-foreground-muted mb-2">选择 AI 聊天服务提供方</div>
      <NSelect
        :value="props.config.provider"
        :options="providerOptions"
        size="small"
        class="w-48"
        @update:value="updateProvider"
      />
    </div>

    <!-- OpenAI API Settings -->
    <div v-if="props.config.provider === 'openai'" class="space-y-4 p-4 rounded-lg bg-panel-muted/50">
      <div class="text-sm font-medium text-accent">OpenAI API 配置</div>
      
      <div class="grid gap-4">
        <div class="space-y-1">
          <div class="text-sm">Base URL</div>
          <div class="text-xs text-foreground-muted">API 服务器地址</div>
          <NInput
            :value="props.config.openai.baseUrl"
            size="small"
            placeholder="https://api.openai.com/v1"
            @update:value="(v) => updateOpenaiField('baseUrl', v)"
          />
        </div>

        <div class="space-y-1">
          <div class="text-sm">Chat Path</div>
          <div class="text-xs text-foreground-muted">聊天接口路径</div>
          <NInput
            :value="props.config.openai.chatPath"
            size="small"
            placeholder="/chat/completions"
            @update:value="(v) => updateOpenaiField('chatPath', v)"
          />
        </div>

        <div class="space-y-1">
          <div class="text-sm">Model</div>
          <div class="text-xs text-foreground-muted">模型名称</div>
          <NInput
            :value="props.config.openai.model"
            size="small"
            placeholder="gpt-4"
            @update:value="(v) => updateOpenaiField('model', v)"
          />
        </div>

        <div class="space-y-1">
          <div class="text-sm">API Key</div>
          <div class="text-xs text-foreground-muted">API 密钥</div>
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
      <div class="text-sm font-medium text-accent">ACP 配置</div>
      
      <NAlert type="info" :bordered="false" class="mb-4">
        ACP (Agent Client Protocol) 需要安装 codex-acp 命令行工具
      </NAlert>

      <div class="space-y-1">
        <div class="text-sm">codex-acp 路径</div>
        <div class="text-xs text-foreground-muted">留空使用系统 PATH 中的 codex-acp</div>
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
