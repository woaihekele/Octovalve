<script setup lang="ts">
import { ref, computed } from 'vue';
import type { ToolCall } from '../types';

const props = defineProps<{
  toolCall: ToolCall;
}>();

const isExpanded = ref(false);

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

const statusLabel = computed(() => {
  switch (props.toolCall.status) {
    case 'running':
      return '执行中';
    case 'completed':
      return '已完成';
    case 'failed':
      return '失败';
    default:
      return '等待中';
  }
});
</script>

<template>
  <div class="tool-call-card" :class="statusClass">
    <div class="tool-badge">
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>
      </svg>
      <span>工具调用</span>
    </div>
    <div class="tool-content">
      <div class="tool-name">{{ toolCall.name }}</div>
      <div class="tool-status">
        <span class="tool-status-dot"></span>
        <span class="tool-status-text">{{ statusLabel }}</span>
      </div>
    </div>
    <button 
      v-if="toolCall.result" 
      class="tool-toggle" 
      @click="isExpanded = !isExpanded"
    >
      <svg 
        width="14" height="14" viewBox="0 0 24 24" 
        fill="none" stroke="currentColor" stroke-width="2"
        :class="{ 'tool-toggle-icon--open': isExpanded }"
      >
        <polyline points="6 9 12 15 18 9"/>
      </svg>
      {{ isExpanded ? '收起输出' : '查看输出' }}
    </button>
    <div v-if="isExpanded && toolCall.result" class="tool-output">
      <pre>{{ toolCall.result }}</pre>
    </div>
  </div>
</template>

<style scoped>
.tool-call-card {
  margin: 8px 0;
  border-radius: 12px;
  border: 1px solid rgba(99, 102, 241, 0.2);
  background: rgb(var(--color-panel));
  overflow: hidden;
  font-size: 13px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.06);
  animation: cardSlideIn 0.3s ease-out;
}

.tool-badge {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 12px;
  background: linear-gradient(135deg, rgba(99, 102, 241, 0.08) 0%, rgba(139, 92, 246, 0.08) 100%);
  border-bottom: 1px solid rgba(99, 102, 241, 0.1);
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: #6366f1;
}

.tool-content {
  padding: 10px 12px;
}

.tool-name {
  font-weight: 600;
  font-size: 14px;
  color: rgb(var(--color-text));
  margin-bottom: 4px;
}

.tool-status {
  display: flex;
  align-items: center;
  gap: 6px;
}

.tool-status-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: rgb(var(--color-text-muted));
}

.tool-status-text {
  font-size: 12px;
  color: rgb(var(--color-text-muted));
}

.tool-toggle {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  width: 100%;
  padding: 8px 12px;
  border: none;
  border-top: 1px solid rgba(0, 0, 0, 0.06);
  background: rgb(var(--color-panel-muted));
  color: rgb(var(--color-text-muted));
  font-size: 12px;
  cursor: pointer;
  transition: all 0.15s;
}

.tool-toggle:hover {
  background: rgb(var(--color-panel));
  color: rgb(var(--color-text));
}

.tool-toggle svg {
  transition: transform 0.2s;
}

.tool-toggle-icon--open {
  transform: rotate(180deg);
}

.tool-output {
  border-top: 1px solid rgba(0, 0, 0, 0.06);
  padding: 10px 12px;
  background: rgb(var(--color-panel-muted));
  max-height: 200px;
  overflow: auto;
}

.tool-output pre {
  margin: 0;
  font-family: 'SF Mono', 'Monaco', 'Consolas', monospace;
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-all;
  color: rgb(var(--color-text));
}

/* Status variations */
.tool-status-running {
  border-color: rgba(245, 158, 11, 0.3);
}

.tool-status-running .tool-status-dot {
  background: #f59e0b;
  animation: pulse 1.5s ease-in-out infinite;
}

.tool-status-running .tool-badge {
  background: linear-gradient(135deg, rgba(245, 158, 11, 0.1) 0%, rgba(251, 191, 36, 0.1) 100%);
  color: #d97706;
}

.tool-status-completed {
  border-color: rgba(34, 197, 94, 0.3);
}

.tool-status-completed .tool-status-dot {
  background: #22c55e;
}

.tool-status-completed .tool-badge {
  background: linear-gradient(135deg, rgba(34, 197, 94, 0.1) 0%, rgba(74, 222, 128, 0.1) 100%);
  color: #16a34a;
}

.tool-status-failed {
  border-color: rgba(239, 68, 68, 0.3);
}

.tool-status-failed .tool-status-dot {
  background: #ef4444;
}

.tool-status-failed .tool-badge {
  background: linear-gradient(135deg, rgba(239, 68, 68, 0.1) 0%, rgba(248, 113, 113, 0.1) 100%);
  color: #dc2626;
}

@keyframes cardSlideIn {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes pulse {
  0%, 100% {
    opacity: 1;
  }
  50% {
    opacity: 0.5;
  }
}
</style>
