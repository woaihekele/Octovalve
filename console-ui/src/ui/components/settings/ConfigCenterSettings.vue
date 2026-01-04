<script setup lang="ts">
import { computed } from 'vue';
import { NButton, NSelect, NSpin } from 'naive-ui';
import type { SelectOption } from 'naive-ui';
import { useI18n } from 'vue-i18n';
import MonacoEditor from '../MonacoEditor.vue';
import type { ConfigFilePayload } from '../../../shared/types';
import type { ResolvedTheme } from '../../../shared/theme';

const props = defineProps<{
  configLoading: boolean;
  configBusy: boolean;
  logModalOpen: boolean;
  selectedProfile: string | null;
  profileOptions: SelectOption[];
  canDeleteProfile: boolean;
  highlight?: boolean;
  proxyConfig: ConfigFilePayload | null;
  brokerConfig: ConfigFilePayload | null;
  proxyConfigText: string;
  brokerConfigText: string;
  proxyDirty: boolean;
  brokerDirty: boolean;
  activeProfile: string | null;
  resolvedTheme: ResolvedTheme;
}>();

const emit = defineEmits<{
  (e: 'request-profile-change', value: string | null): void;
  (e: 'open-create-profile'): void;
  (e: 'open-delete-profile'): void;
  (e: 'request-refresh'): void;
  (e: 'close'): void;
  (e: 'save'): void;
  (e: 'apply'): void;
  (e: 'update:proxyConfigText', value: string): void;
  (e: 'update:brokerConfigText', value: string): void;
}>();

const { t } = useI18n();

const proxyConfigModel = computed({
  get: () => props.proxyConfigText,
  set: (value: string) => emit('update:proxyConfigText', value),
});

const brokerConfigModel = computed({
  get: () => props.brokerConfigText,
  set: (value: string) => emit('update:brokerConfigText', value),
});

const statusText = computed(() => {
  return t('settings.config.status', {
    active: props.activeProfile || '-',
    selected: props.selectedProfile || '-',
    proxy: props.proxyDirty ? t('settings.config.changed') : t('settings.config.unchanged'),
    broker: props.brokerDirty ? t('settings.config.changed') : t('settings.config.unchanged'),
  });
});
</script>

<template>
  <div
    class="flex flex-col gap-4 min-h-0 flex-1 transition"
    :class="props.highlight ? 'ring-1 ring-accent/60 rounded-lg' : ''"
  >
    <div v-if="props.configLoading" class="flex items-center gap-2 text-sm text-foreground-muted">
      <NSpin size="small" />
      <span>{{ $t('settings.config.loading') }}</span>
    </div>
    <div v-else class="flex flex-col gap-3 min-h-0 flex-1">
      <div class="flex flex-wrap items-center justify-between gap-3 text-sm">
        <div>
          <div class="font-medium">{{ $t('settings.profile.label') }}</div>
          <div class="text-xs text-foreground-muted">{{ $t('settings.profile.help') }}</div>
        </div>
        <div class="flex items-center gap-2">
          <NSelect
            :value="props.selectedProfile"
            :options="props.profileOptions"
            size="small"
            class="w-40"
            :placeholder="$t('settings.profile.placeholder')"
            :disabled="props.configBusy || props.logModalOpen || props.configLoading"
            @update:value="(v) => emit('request-profile-change', v as string | null)"
          />
          <NButton size="small" :disabled="props.configBusy || props.logModalOpen || props.configLoading" @click="emit('open-create-profile')">
            {{ $t('common.create') }}
          </NButton>
          <NButton
            size="small"
            quaternary
            :disabled="props.configBusy || props.logModalOpen || props.configLoading || !props.canDeleteProfile"
            @click="emit('open-delete-profile')"
          >
            {{ $t('common.delete') }}
          </NButton>
        </div>
      </div>

      <div class="grid grid-cols-1 lg:grid-cols-2 gap-4 min-h-0 flex-1">
        <div class="flex flex-col gap-2 min-h-0 flex-1">
          <div class="flex items-center justify-between text-sm">
            <div>
              <div class="font-medium">{{ $t('settings.config.proxyTitle') }}</div>
              <div class="text-xs text-foreground-muted break-all">{{ props.proxyConfig?.path }}</div>
            </div>
            <span v-if="props.proxyConfig && !props.proxyConfig.exists" class="text-xs text-warning">{{ $t('settings.config.missing') }}</span>
          </div>
          <div class="flex-1 min-h-0">
            <MonacoEditor v-model="proxyConfigModel" language="toml" height="100%" :theme="props.resolvedTheme" />
          </div>
        </div>

        <div class="flex flex-col gap-2 min-h-0 flex-1">
          <div class="flex items-center justify-between text-sm">
            <div>
              <div class="font-medium">{{ $t('settings.config.brokerTitle') }}</div>
              <div class="text-xs text-foreground-muted break-all">{{ props.brokerConfig?.path }}</div>
            </div>
          </div>
          <div class="flex-1 min-h-0">
            <MonacoEditor v-model="brokerConfigModel" language="toml" height="100%" :theme="props.resolvedTheme" />
          </div>
        </div>
      </div>
    </div>

    <div class="mt-4 flex items-center justify-between gap-3">
      <div class="text-xs text-foreground-muted">{{ statusText }}</div>
      <div class="flex items-center gap-2">
        <NButton
          quaternary
          :disabled="props.configBusy || props.logModalOpen || props.configLoading"
          @click="emit('request-refresh')"
        >
          {{ $t('common.refresh') }}
        </NButton>
        <NButton :disabled="props.configBusy || props.logModalOpen" @click="emit('close')">
          {{ $t('common.cancel') }}
        </NButton>
        <NButton :disabled="props.configBusy || props.logModalOpen || props.configLoading" @click="emit('save')">
          {{ $t('common.save') }}
        </NButton>
        <NButton type="primary" :disabled="props.configBusy || props.logModalOpen || props.configLoading" @click="emit('apply')">
          {{ $t('common.apply') }}
        </NButton>
      </div>
    </div>
  </div>
</template>
