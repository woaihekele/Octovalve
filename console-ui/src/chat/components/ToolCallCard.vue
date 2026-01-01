<script setup lang="ts">
import { ref, computed } from 'vue';
import type { ToolCall } from '../types';

const props = defineProps<{
  toolCall: ToolCall;
}>();

const isExpanded = ref(false);

const statusIcon = computed(() => {
  switch (props.toolCall.status) {
    case 'running':
      return '⏳';
    case 'completed':
      return '✅';
    case 'failed':
      return '❌';
    default:
      return '⏳';
  }
});

const statusClass = computed(() => {
  switch (props.toolCall.status) {
    case 'running':
      return 'tool-status-running';
    case 'completed':
      return 'tool-status-completed';
    case 'failed':
      return 'tool-status-failed';
    default:
      return 'tool-status-pending';
  }
});
</script>

<template>
  <div class="tool-call-card" :class="statusClass">
    <div class="tool-header" @click="isExpanded = !isExpanded">
      <span class="tool-icon">{{ statusIcon }}</span>
      <span class="tool-name">{{ toolCall.name }}</span>
      <span class="expand-icon">{{ isExpanded ? '▼' : '▶' }}</span>
    </div>
    <div v-if="isExpanded && toolCall.result" class="tool-output">
      <pre>{{ toolCall.result }}</pre>
    </div>
  </div>
</template>

<style scoped>
.tool-call-card {
  margin: 8px 0;
  border-radius: 8px;
  border: 1px solid var(--border-color, #e0e0e0);
  background: var(--bg-secondary, #f8f9fa);
  overflow: hidden;
  font-size: 13px;
}

.tool-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  cursor: pointer;
  user-select: none;
}

.tool-header:hover {
  background: var(--bg-hover, rgba(0, 0, 0, 0.03));
}

.tool-icon {
  font-size: 14px;
}

.tool-name {
  flex: 1;
  font-weight: 500;
  color: var(--text-primary, #333);
}

.expand-icon {
  font-size: 10px;
  color: var(--text-secondary, #666);
}

.tool-output {
  border-top: 1px solid var(--border-color, #e0e0e0);
  padding: 8px 12px;
  background: var(--bg-code, #f5f5f5);
  max-height: 200px;
  overflow: auto;
}

.tool-output pre {
  margin: 0;
  font-family: 'SF Mono', 'Monaco', 'Consolas', monospace;
  font-size: 12px;
  white-space: pre-wrap;
  word-break: break-all;
  color: var(--text-code, #333);
}

.tool-status-running {
  border-color: #ffc107;
}

.tool-status-running .tool-header {
  background: rgba(255, 193, 7, 0.1);
}

.tool-status-completed {
  border-color: #28a745;
}

.tool-status-completed .tool-header {
  background: rgba(40, 167, 69, 0.1);
}

.tool-status-failed {
  border-color: #dc3545;
}

.tool-status-failed .tool-header {
  background: rgba(220, 53, 69, 0.1);
}
</style>
