<script setup lang="ts">
import { ref, computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { ToolCall } from '../types';

const props = defineProps<{
  toolCall: ToolCall;
}>();

const isExpanded = ref(false);
const { t } = useI18n();

const canExpand = computed(() => {
  const args = props.toolCall.arguments;
  const hasArgs = args && Object.keys(args).length > 0;
  return hasArgs || Boolean(props.toolCall.result);
});

const argsSummary = computed(() => {
  const args = props.toolCall.arguments;
  if (!args || Object.keys(args).length === 0) {
    return '';
  }

  const parts: string[] = [];
  for (const [k, v] of Object.entries(args)) {
    if (parts.length >= 3) {
      break;
    }

    const formatted = (() => {
      if (v === null) {
        return 'null';
      }
      if (typeof v === 'string') {
        const trimmed = v.length > 48 ? `${v.slice(0, 48)}…` : v;
        return JSON.stringify(trimmed);
      }
      if (typeof v === 'number' || typeof v === 'boolean') {
        return String(v);
      }

      try {
        const s = JSON.stringify(v);
        if (!s) {
          return '[object]';
        }
        return s.length > 48 ? `${s.slice(0, 48)}…` : s;
      } catch {
        return '[object]';
      }
    })();

    parts.push(`${k}: ${formatted}`);
  }

  const omitted = Math.max(0, Object.keys(args).length - parts.length);
  const summary = parts.join('\n');
  return omitted > 0 ? `${summary}\n+${omitted}` : summary;
});

const formattedArgs = computed(() => {
  const args = props.toolCall.arguments;
  if (!args || Object.keys(args).length === 0) {
    return '';
  }
  try {
    return JSON.stringify(args, null, 2);
  } catch {
    return String(args);
  }
});

const outputText = computed(() => {
  if (props.toolCall.result) {
    return props.toolCall.result;
  }
  if (props.toolCall.status === 'running' || props.toolCall.status === 'pending') {
    return t('chat.tool.running');
  }
  return t('chat.tool.noOutput');
});

const statusLabel = computed(() => {
  switch (props.toolCall.status) {
    case 'pending':
      return t('chat.tool.status.pending');
    case 'running':
      return t('chat.tool.status.running');
    case 'completed':
      return t('chat.tool.status.completed');
    case 'failed':
      return t('chat.tool.status.failed');
    case 'cancelled':
      return t('chat.tool.status.cancelled');
    default:
      return props.toolCall.status;
  }
});

const statusPillClass = computed(() => {
  switch (props.toolCall.status) {
    case 'completed':
      return 'bg-success/20 text-success';
    case 'failed':
    case 'cancelled':
      return 'bg-danger/20 text-danger';
    case 'running':
      return 'bg-warning/20 text-warning';
    default:
      return 'bg-panel-muted/40 text-foreground-muted';
  }
});

const statusClass = computed(() => {
  switch (props.toolCall.status) {
    case 'running':
      return 'tool-status-running';
    case 'completed':
      return 'tool-status-completed';
    case 'failed':
    case 'cancelled':
      return 'tool-status-failed';
    default:
      return 'tool-status-pending';
  }
});
</script>

<template>
  <div class="tool-call-card" :class="statusClass">
    <div class="tool-badge">
      <svg class="tool-badge__icon" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>
      </svg>
      <span class="tool-badge__label">{{ $t('chat.tool.label') }}</span>
      <span class="tool-badge__meta">
        <span class="tool-badge__name">{{ toolCall.name }}</span>
        <span v-if="argsSummary" class="tool-badge__args">{{ argsSummary }}</span>
      </span>
      <span class="tool-status-pill" :class="statusPillClass">{{ statusLabel }}</span>
    </div>
    <button 
      v-if="canExpand" 
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
      {{ isExpanded ? $t('chat.tool.collapse') : $t('chat.tool.expand') }}
    </button>
    <div v-if="isExpanded && canExpand" class="tool-details">
      <div class="tool-detail">
        <div class="tool-detail__title">{{ $t('chat.tool.input') }}</div>
        <pre v-if="formattedArgs">{{ formattedArgs }}</pre>
        <div v-else class="tool-detail__empty">{{ $t('chat.tool.noInput') }}</div>
      </div>
      <div class="tool-detail">
        <div class="tool-detail__title">{{ $t('chat.tool.output') }}</div>
        <pre>{{ outputText }}</pre>
      </div>
    </div>
  </div>
</template>

<style scoped>
.tool-call-card {
  margin: 8px 0;
  border-radius: 12px;
  border: 1px solid rgb(var(--color-border));
  background: rgb(var(--color-panel));
  overflow: hidden;
  font-size: 13px;
  box-shadow: none;
  animation: cardSlideIn 0.3s ease-out;
}

.tool-badge {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 12px;
  background: rgb(var(--color-panel-muted));
  border-bottom: 1px solid rgb(var(--color-border));
  font-size: 12px;
  font-weight: 500;
  color: rgb(var(--color-text-muted));
  gap: 8px;
  flex-wrap: nowrap;
}

.tool-badge__icon {
  color: rgb(var(--color-accent));
  flex: 0 0 auto;
}

.tool-badge__label {
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: rgb(var(--color-text-muted));
}

.tool-badge__meta {
  display: inline-flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
  min-width: 0;
  flex: 1 1 auto;
  overflow: hidden;
}

.tool-badge__name {
  color: rgb(var(--color-text));
  font-weight: 600;
  font-size: 13px;
  font-family: 'SF Mono', 'Monaco', 'Consolas', monospace;
}

.tool-badge__args {
  color: rgb(var(--color-text-muted));
  font-weight: 400;
  font-size: 12px;
  font-family: 'SF Mono', 'Monaco', 'Consolas', monospace;
  white-space: pre-wrap;
  word-break: break-all;
}

.tool-status-pill {
  flex: 0 0 auto;
  margin-left: auto;
  font-size: 12px;
  padding: 2px 8px;
  border-radius: 8px;
  font-style: italic;
  white-space: nowrap;
}

.tool-toggle {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  width: 100%;
  padding: 8px 12px;
  border: none;
  border-top: 1px solid rgb(var(--color-border));
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

.tool-details {
  border-top: 1px solid rgb(var(--color-border));
  padding: 10px 12px;
  background: rgb(var(--color-panel-muted));
  max-height: 240px;
  overflow: auto;
}

.tool-detail {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.tool-detail + .tool-detail {
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px dashed rgb(var(--color-border));
}

.tool-detail__title {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: rgb(var(--color-text-muted));
}

.tool-detail__empty {
  font-size: 12px;
  color: rgb(var(--color-text-muted));
}

.tool-details pre {
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
  border-color: transparent;
  box-shadow:
    0 0 0 1px rgb(var(--color-warning) / 0.55),
    0 0 12px rgb(var(--color-warning) / 0.25);
}

.tool-status-running .tool-status-dot {
  background: rgb(var(--color-warning));
  animation: pulse 1.5s ease-in-out infinite;
}

.tool-status-running .tool-badge {
  background: rgb(var(--color-panel-muted));
  color: rgb(var(--color-text-muted));
}

.tool-status-completed {
  border-color: transparent;
  box-shadow:
    0 0 0 1px rgb(var(--color-success) / 0.55),
    0 0 12px rgb(var(--color-success) / 0.25);
}

.tool-status-completed .tool-status-dot {
  background: rgb(var(--color-success));
}

.tool-status-completed .tool-badge {
  background: rgb(var(--color-panel-muted));
  color: rgb(var(--color-text-muted));
}

.tool-status-failed {
  border-color: transparent;
  box-shadow:
    0 0 0 1px rgb(var(--color-danger) / 0.55),
    0 0 12px rgb(var(--color-danger) / 0.25);
}

.tool-status-pending {
  border-color: transparent;
  box-shadow:
    0 0 0 1px rgb(var(--color-accent) / 0.55),
    0 0 12px rgb(var(--color-accent) / 0.25);
}

.tool-status-failed .tool-status-dot {
  background: rgb(var(--color-danger));
}

.tool-status-failed .tool-badge {
  background: rgb(var(--color-panel-muted));
  color: rgb(var(--color-text-muted));
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
