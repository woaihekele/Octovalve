<script setup lang="ts">
import { computed } from 'vue';
import { NSelect, type SelectOption } from 'naive-ui';
import { useI18n } from 'vue-i18n';
import type { ProfileSummary, TargetInfo } from '../../shared/types';

type ConnectionState = 'connected' | 'connecting' | 'disconnected';

const props = defineProps<{
  targets: TargetInfo[];
  selectedTargetName: string | null;
  pendingTotal: number;
  connectionState: ConnectionState;
  profiles: ProfileSummary[];
  activeProfile: string | null;
  profilesEnabled: boolean;
  profileLoading?: boolean;
  profileSwitching?: boolean;
  sidebarWidth?: number;
  profileSelectWidth?: number;
}>();

const emit = defineEmits<{
  (e: 'select', name: string): void;
  (e: 'open-settings'): void;
  (e: 'switch-profile', name: string): void;
}>();

const { t } = useI18n();
const isMacPlatform = computed(() => {
  if (typeof navigator === 'undefined') {
    return false;
  }
  const platform =
    (navigator as { userAgentData?: { platform?: string } }).userAgentData?.platform ||
    navigator.platform ||
    navigator.userAgent;
  return /mac|iphone|ipad|ipod/i.test(platform);
});
const profileOptions = computed<SelectOption[]>(() =>
  props.profiles.map((profile) => ({ label: profile.name, value: profile.name }))
);
const sidebarStyle = computed(() => ({
  width: props.sidebarWidth ? `${props.sidebarWidth}px` : undefined,
  minWidth: props.sidebarWidth ? `${props.sidebarWidth}px` : undefined,
  ['--sidebar-profile-width' as string]: props.profileSelectWidth
    ? `${props.profileSelectWidth}px`
    : undefined,
}));

function resolveStatus(target: TargetInfo) {
  if (target.status === 'ready') {
    return 'ready';
  }
  if (props.connectionState === 'connecting') {
    return 'connecting';
  }
  if (!target.last_seen && !target.last_error) {
    return 'connecting';
  }
  return 'down';
}

function statusLabel(target: TargetInfo) {
  const status = resolveStatus(target);
  if (status === 'ready') return t('status.ready');
  if (status === 'connecting') return t('status.connecting');
  return t('status.down');
}

function statusClass(target: TargetInfo) {
  const status = resolveStatus(target);
  if (status === 'ready') return 'bg-success';
  if (status === 'connecting') return 'bg-warning';
  return 'bg-danger';
}

function resolveTargetHost(target: TargetInfo) {
  const ssh = target.ssh?.trim();
  if (!ssh) {
    return target.desc;
  }
  const at = ssh.lastIndexOf('@');
  if (at > 0 && at < ssh.length - 1) {
    return ssh.slice(at + 1);
  }
  return ssh;
}
</script>

<template>
  <aside class="w-72 shrink-0 bg-panel border-r border-border flex flex-col h-full" :style="sidebarStyle">
    <div
      class="p-4 border-b border-border flex items-center gap-2"
      :data-tauri-drag-region="isMacPlatform ? true : undefined"
    >
      <div class="w-2.5 h-2.5 rounded-full bg-accent"></div>
      <h1 class="font-semibold text-lg tracking-tight">Octovalve</h1>
    </div>

    <div class="flex-1 overflow-y-auto scrollbar-chat p-2 space-y-1">
      <button
        v-for="target in props.targets"
        :key="target.name"
        type="button"
        @click="emit('select', target.name)"
        class="w-full text-left p-3 rounded-lg transition-colors flex items-center justify-between"
        :class="props.selectedTargetName === target.name ? 'bg-panel-muted border border-border' : 'hover:bg-panel-muted/50 border border-transparent'"
      >
        <div class="flex flex-col min-w-0">
          <div class="flex items-center gap-2">
            <span class="font-medium text-sm truncate text-foreground">{{ target.name }}</span>
            <span v-if="target.is_default" class="text-[10px] px-1.5 py-0.5 rounded bg-accent/20 text-accent">
              {{ $t('console.default') }}
            </span>
          </div>
          <div class="flex items-center gap-2 mt-1 text-xs text-foreground-muted">
            <span class="truncate">
              {{ resolveTargetHost(target) }}
            </span>
          </div>
          <div class="flex items-center gap-2 mt-1 text-xs text-foreground-muted">
            <span class="inline-flex items-center gap-1">
              <span class="h-2 w-2 rounded-full" :class="statusClass(target)"></span>
              <span class="capitalize">{{ statusLabel(target) }}</span>
            </span>
          </div>
        </div>

        <div
          v-if="target.pending_count > 0"
          class="bg-danger text-white text-xs font-semibold px-2 py-0.5 rounded-full min-w-[20px] text-center shadow-sm shadow-danger/40"
        >
          {{ target.pending_count }}
        </div>
      </button>
    </div>

    <div class="p-4 border-t border-border flex items-center gap-2 text-xs text-foreground-muted">
      <n-select
        v-if="profilesEnabled"
        :value="activeProfile"
        :options="profileOptions"
        size="small"
        :consistent-menu-width="false"
        :loading="profileLoading"
        :disabled="profileLoading || profileSwitching || profileOptions.length === 0"
        :placeholder="$t('settings.profile.placeholder')"
        class="sidebar__profile-select"
        @update:value="(value) => emit('switch-profile', value as string)"
      />
      <button
        class="ml-auto p-2 rounded-full border border-border text-foreground-muted hover:text-foreground hover:border-accent/40 transition-colors"
        @click="emit('open-settings')"
        :aria-label="$t('settings.title')"
        :title="$t('settings.title')"
      >
        <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3"></circle>
          <path
            d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33 1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82 1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"
          ></path>
        </svg>
      </button>
    </div>
  </aside>
</template>

<style scoped>
.sidebar__profile-select {
  width: var(--sidebar-profile-width, 150px);
}

.sidebar__profile-select :deep(.n-base-selection) {
  --n-height: 28px;
  --n-font-size: 12px;
  --n-border: 1px solid rgb(var(--color-border));
  --n-border-hover: 1px solid rgba(var(--color-accent), 0.5);
  --n-border-active: 1px solid rgb(var(--color-accent));
  --n-border-focus: 1px solid rgb(var(--color-accent));
  --n-box-shadow-focus: 0 0 0 2px rgba(99, 102, 241, 0.18);
}
</style>
