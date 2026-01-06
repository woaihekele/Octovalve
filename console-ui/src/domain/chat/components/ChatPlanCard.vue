<template>
  <div class="chat-plan">
    <button type="button" class="chat-plan__header" @click="toggleOpen">
      <div class="chat-plan__title">
        <span class="chat-plan__title-text">{{ $t('chat.plan.title') }}</span>
        <span class="chat-plan__count">{{ countLabel }}</span>
      </div>
      <svg
        class="chat-plan__caret"
        :class="{ 'chat-plan__caret--open': isOpen }"
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
      >
        <polyline points="6 9 12 15 18 9" />
      </svg>
    </button>
    <div v-show="isOpen" class="chat-plan__body">
      <ol class="chat-plan__list">
        <li v-for="(entry, index) in entries" :key="index" class="chat-plan__item">
          <span class="chat-plan__index">{{ index + 1 }}</span>
          <span :class="['chat-plan__content', contentClass(entry.status)]">
            {{ entry.content }}
          </span>
          <span :class="['chat-plan__status', statusClass(entry.status)]">
            {{ statusLabel(entry.status) }}
          </span>
        </li>
      </ol>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import type { PlanEntry, PlanEntryStatus } from '../types';

const props = defineProps<{
  entries: PlanEntry[];
}>();

const { t } = useI18n();
const isOpen = ref(true);

const countLabel = computed(() => t('chat.plan.count', { count: props.entries.length }));

const statusLabel = (status: PlanEntryStatus): string => {
  switch (status) {
    case 'pending':
      return t('chat.plan.status.pending');
    case 'in_progress':
      return t('chat.plan.status.inProgress');
    case 'completed':
      return t('chat.plan.status.completed');
    default:
      return status;
  }
};

const statusClass = (status: PlanEntryStatus): string => {
  return `chat-plan__status--${status.replace('_', '-')}`;
};

const contentClass = (status: PlanEntryStatus): string => {
  return status === 'completed' ? 'chat-plan__content--completed' : '';
};

function toggleOpen() {
  isOpen.value = !isOpen.value;
}
</script>

<style scoped>
.chat-plan {
  margin: 8px 16px 12px;
  border-radius: 12px;
  border: 1px solid rgb(var(--color-border));
  background: rgb(var(--color-panel));
  overflow: hidden;
}

.chat-plan__header {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 12px;
  background: rgb(var(--color-panel-muted));
  border: none;
  color: rgb(var(--color-text));
  text-align: left;
  cursor: pointer;
}

.chat-plan__title {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.chat-plan__title-text {
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.2px;
}

.chat-plan__count {
  font-size: 12px;
  color: rgb(var(--color-text-muted));
}

.chat-plan__caret {
  transition: transform 0.2s ease;
}

.chat-plan__caret--open {
  transform: rotate(180deg);
}

.chat-plan__body {
  padding: 6px 12px 12px;
}

.chat-plan__list {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.chat-plan__item {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: 13px;
  color: rgb(var(--color-text));
}

.chat-plan__index {
  width: 22px;
  height: 22px;
  border-radius: 999px;
  border: 1px solid rgb(var(--color-border));
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  color: rgb(var(--color-text-muted));
  flex-shrink: 0;
}

.chat-plan__content {
  flex: 1 1 auto;
  min-width: 0;
  white-space: pre-wrap;
  word-break: break-word;
  line-height: 1.5;
}

.chat-plan__content--completed {
  text-decoration: line-through;
  color: rgb(var(--color-text-muted));
}

.chat-plan__status {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: rgb(var(--color-text-muted));
  line-height: 1.5;
}

.chat-plan__status::before {
  content: '';
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background: rgb(var(--color-border));
}

.chat-plan__status--in-progress {
  color: rgb(var(--color-warning));
}

.chat-plan__status--in-progress::before {
  background: rgb(var(--color-warning));
}

.chat-plan__status--completed {
  color: rgb(var(--color-success));
}

.chat-plan__status--completed::before {
  background: rgb(var(--color-success));
}
</style>
