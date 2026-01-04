<template>
  <Sidebar
    :targets="targets"
    :selected-target-name="selectedTargetName"
    :pending-total="pendingTotal"
    :connection-state="connectionState"
    @select="emit('select-target', $event)"
    @open-settings="emit('open-settings')"
  />

  <div class="flex-1 flex min-w-0 min-h-0 overflow-hidden">
    <div class="flex-1 flex flex-col min-w-0 min-h-0 relative">
      <div class="absolute top-4 right-4 z-20 flex items-center gap-3">
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
          :aria-label="isChatOpen ? '收起 AI 助手' : '展开 AI 助手'"
          :title="isChatOpen ? '收起 AI 助手' : '展开 AI 助手'"
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
          @refresh-risk="emit('refresh-risk', $event)"
          @open-terminal="emit('open-terminal')"
          @close-terminal="emit('close-terminal')"
          @toggle-chat="emit('toggle-chat')"
        >
          <template #terminal>
            <div class="flex flex-col min-h-0 h-full">
              <div v-if="selectedTerminalEntry" class="pt-1 pb-0 bg-surface">
                <n-tabs
                  :value="activeTerminalTabId"
                  type="card"
                  size="small"
                  addable
                  closable
                  class="min-w-0 terminal-tabs"
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
              <div class="flex-1 min-h-0 relative">
                <template v-for="entry in terminalEntries" :key="entry.target.name">
                  <TerminalPanel
                    v-for="tab in entry.state.tabs"
                    :key="tab.id"
                    :target="entry.target"
                    :theme="resolvedTheme"
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
          请选择目标开始操作。
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { NTabPane, NTabs } from 'naive-ui';
import type { ResolvedTheme } from '../../shared/theme';
import type { AiRiskEntry, AppSettings, ServiceSnapshot, TargetInfo } from '../../shared/types';
import Sidebar from './Sidebar.vue';
import TargetView from './TargetView.vue';
import TerminalPanel from './TerminalPanel.vue';

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

defineProps<{
  targets: TargetInfo[];
  selectedTargetName: string | null;
  pendingTotal: number;
  connectionState: 'connected' | 'connecting' | 'disconnected';
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
  resolvedTheme: ResolvedTheme;
}>();

const emit = defineEmits<{
  (e: 'select-target', value: string): void;
  (e: 'open-settings'): void;
  (e: 'toggle-chat'): void;
  (e: 'approve', id: string): void;
  (e: 'deny', id: string): void;
  (e: 'refresh-risk', payload: { target: string; id: string }): void;
  (e: 'open-terminal'): void;
  (e: 'close-terminal'): void;
  (e: 'terminal-add'): void;
  (e: 'terminal-close', value: string | number): void;
  (e: 'terminal-activate', value: string | number): void;
}>();

</script>
