<template>
  <Sidebar
    :targets="targets"
    :selected-target-name="selectedTargetName"
    :pending-total="pendingTotal"
    :connection-state="connectionState"
    :profiles="profiles"
    :active-profile="activeProfile"
    :profiles-enabled="profilesEnabled"
    :profile-loading="profileLoading"
    :profile-switching="profileSwitching"
    :sidebar-width="sidebarWidth"
    @select="emit('select-target', $event)"
    @open-settings="emit('open-settings')"
    @switch-profile="emit('switch-profile', $event)"
  />

  <div class="flex-1 flex min-w-0 min-h-0 overflow-hidden" :style="{ minWidth: `${TARGET_MIN_MAIN_WIDTH}px` }">
    <div class="flex-1 flex flex-col min-w-0 min-h-0">
      <!-- 用正常布局占位，避免 “正在启动...” 浮层遮挡 TargetView 顶部按钮 -->
      <div class="shrink-0 pt-4 px-4 flex items-center justify-end gap-3">
        <span
          v-if="consoleBanner"
          class="text-xs px-2 py-1 rounded border"
          :class="consoleBanner.kind === 'error'
            ? 'bg-danger/20 text-danger border-danger/30'
            : 'bg-warning/20 text-warning border-warning/30'"
        >
          {{ consoleBanner.message }}
        </span>

        <button
          v-if="!selectedTarget"
          class="p-2 rounded border transition-colors bg-panel/60 text-foreground border-border hover:border-accent/40"
          @click="emit('toggle-chat')"
          :aria-label="isChatOpen ? $t('chat.toggle.close') : $t('chat.toggle.open')"
          :title="isChatOpen ? $t('chat.toggle.close') : $t('chat.toggle.open')"
        >
          <svg
            class="h-4 w-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <line x1="12" y1="3" x2="12" y2="6" />
            <circle cx="12" cy="3" r="1" />
            <rect x="4" y="6" width="16" height="12" rx="3" />
            <circle cx="9" cy="12" r="1.2" fill="currentColor" stroke="none" />
            <circle cx="15" cy="12" r="1.2" fill="currentColor" stroke="none" />
            <line x1="9" y1="15" x2="15" y2="15" />
          </svg>
        </button>
      </div>

      <div class="flex-1 min-h-0">
        <TargetView
          v-if="selectedTarget"
          :target="selectedTarget"
          :snapshot="selectedSnapshot"
          :settings="settings"
          :pending-jump-token="pendingJumpToken"
          :terminal-open="selectedTerminalOpen"
          :chat-open="isChatOpen"
          :ai-risk-map="aiRiskMap"
          :ai-enabled="aiEnabled"
          @approve="emit('approve', $event)"
          @deny="emit('deny', $event)"
          @cancel="emit('cancel', $event)"
          @refresh-risk="emit('refresh-risk', $event)"
          @open-terminal="emit('open-terminal')"
          @close-terminal="emit('close-terminal')"
          @toggle-chat="emit('toggle-chat')"
          @open-upload="openUploadModal"
        >
          <template #terminal>
            <div class="flex flex-col min-h-0 h-full">
              <div v-if="selectedTerminalEntry" class="pt-1 pb-0 bg-surface">
                <div class="flex items-center gap-2">
                  <n-tabs
                    :value="activeTerminalTabId"
                    type="card"
                    size="small"
                    addable
                    closable
                    :pane-wrapper-style="{ display: 'none' }"
                    class="min-w-0 flex-1 terminal-tabs"
                    @add="emit('terminal-add')"
                    @close="emit('terminal-close', $event)"
                    @update:value="emit('terminal-activate', $event)"
                  >
                  <n-tab-pane
                    v-for="tab in selectedTerminalEntry.state.tabs"
                    :key="tab.id"
                    :name="tab.id"
                    :tab="tab.label"
                    closable
                  />
                </n-tabs>
              </div>
            </div>
              <div class="flex-1 min-h-0 relative">
                <template v-for="entry in terminalEntries" :key="entry.target.name">
                  <TerminalPanel
                    v-for="tab in entry.state.tabs"
                    :key="tab.id"
                    :ref="setTerminalRef(entry.target.name, tab.id)"
                    :target="entry.target"
                    :theme="resolvedTheme"
                    :terminal-scale="terminalScale"
                    :visible="
                      selectedTerminalOpen &&
                      selectedTargetName === entry.target.name &&
                      entry.state.activeId === tab.id
                    "
                    v-show="
                      selectedTerminalOpen &&
                      selectedTargetName === entry.target.name &&
                      entry.state.activeId === tab.id
                    "
                  />
                </template>
              </div>
            </div>
          </template>
        </TargetView>
        <div v-else class="flex-1 flex items-center justify-center text-foreground-muted">
          {{ $t('console.emptyTarget') }}
        </div>
      </div>
    </div>
  </div>

  <TerminalUploadModal v-model:show="uploadOpen" :target="selectedTarget" />
</template>

<script setup lang="ts">
import { ref } from 'vue';
import type { ComponentPublicInstance } from 'vue';
import { NTabPane, NTabs } from 'naive-ui';
import type { ResolvedTheme } from '../../shared/theme';
import type { AiRiskEntry, AppSettings, ProfileSummary, ServiceSnapshot, TargetInfo } from '../../shared/types';
import Sidebar from './Sidebar.vue';
import TargetView from './TargetView.vue';
import TerminalPanel from './TerminalPanel.vue';
import TerminalUploadModal from './TerminalUploadModal.vue';
import { TARGET_MIN_MAIN_WIDTH } from '../layout';

type TerminalTab = {
  id: string;
  label: string;
  createdAt: number;
};

type TerminalEntry = {
  target: TargetInfo;
  state: {
    tabs: TerminalTab[];
    activeId: string | null;
  };
};

const props = defineProps<{
  targets: TargetInfo[];
  selectedTargetName: string | null;
  pendingTotal: number;
  connectionState: 'connected' | 'connecting' | 'disconnected';
  profiles: ProfileSummary[];
  activeProfile: string | null;
  profilesEnabled: boolean;
  profileLoading: boolean;
  profileSwitching: boolean;
  sidebarWidth?: number;
  selectedTarget: TargetInfo | null;
  selectedSnapshot: ServiceSnapshot | null;
  settings: AppSettings;
  pendingJumpToken: number;
  selectedTerminalOpen: boolean;
  isChatOpen: boolean;
  aiRiskMap: Record<string, AiRiskEntry>;
  aiEnabled: boolean;
  consoleBanner?: { kind: 'error' | 'info'; message: string } | null;
  selectedTerminalEntry: TerminalEntry | null;
  activeTerminalTabId: string | number | undefined;
  terminalEntries: TerminalEntry[];
  terminalScale: number;
  resolvedTheme: ResolvedTheme;
}>();

type TerminalPanelExpose = {
  focus: () => void;
  blur: () => void;
  hasFocus: () => boolean;
};

const terminalRefMap = new Map<string, TerminalPanelExpose>();
const uploadOpen = ref(false);
function terminalKey(targetName: string, tabId: string) {
  return `${targetName}::${tabId}`;
}

function isTerminalPanelExpose(value: unknown): value is TerminalPanelExpose {
  return (
    !!value &&
    typeof (value as TerminalPanelExpose).focus === 'function' &&
    typeof (value as TerminalPanelExpose).blur === 'function' &&
    typeof (value as TerminalPanelExpose).hasFocus === 'function'
  );
}

function setTerminalRef(targetName: string, tabId: string) {
  return (el: Element | ComponentPublicInstance | null) => {
    const key = terminalKey(targetName, tabId);
    if (el && isTerminalPanelExpose(el)) {
      terminalRefMap.set(key, el);
      return;
    }
    terminalRefMap.delete(key);
  };
}

function getActiveTerminalRef(): TerminalPanelExpose | null {
  const entry = props.selectedTerminalEntry;
  if (!entry || !entry.state.activeId) {
    return null;
  }
  const key = terminalKey(entry.target.name, entry.state.activeId);
  return terminalRefMap.get(key) ?? null;
}

function focusActiveTerminal() {
  getActiveTerminalRef()?.focus();
}

function blurActiveTerminal() {
  getActiveTerminalRef()?.blur();
}

function isActiveTerminalFocused() {
  return getActiveTerminalRef()?.hasFocus() ?? false;
}

function openUploadModal() {
  if (!props.selectedTarget || !props.selectedTarget.terminal_available) {
    return;
  }
  uploadOpen.value = true;
}

defineExpose({
  focusActiveTerminal,
  blurActiveTerminal,
  isActiveTerminalFocused,
});

const emit = defineEmits<{
  (e: 'select-target', value: string): void;
  (e: 'open-settings'): void;
  (e: 'switch-profile', value: string): void;
  (e: 'toggle-chat'): void;
  (e: 'approve', id: string): void;
  (e: 'deny', id: string): void;
  (e: 'cancel', id: string): void;
  (e: 'refresh-risk', payload: { target: string; id: string }): void;
  (e: 'open-terminal'): void;
  (e: 'close-terminal'): void;
  (e: 'terminal-add'): void;
  (e: 'terminal-close', value: string | number): void;
  (e: 'terminal-activate', value: string | number): void;
}>();

</script>
