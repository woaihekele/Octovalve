<script setup lang="ts">
import { h, watch } from 'vue';
import { useNotification } from 'naive-ui';
import { useI18n } from 'vue-i18n';

const props = defineProps<{
  payload: { message: string; count?: number; target?: string; type?: 'success' | 'warning' | 'error' | 'info' } | null;
  token: number;
}>();

const emit = defineEmits<{
  (e: 'jump-pending', target: string): void;
}>();

const notification = useNotification();
const { t } = useI18n();

watch(
  () => props.token,
  () => {
    if (!props.payload) {
      return;
    }
    const target = props.payload.target;
    const contentText = props.payload.count ? t('console.notifications.pendingCount', { count: props.payload.count }) : undefined;
    const content = target && contentText
      ? () =>
          h(
            'span',
            {
              style: {
                cursor: 'pointer',
                color: 'rgb(var(--color-accent))',
              },
              onClick: (event: MouseEvent) => {
                event.stopPropagation();
                emit('jump-pending', target);
              },
            },
            contentText
          )
      : contentText;
    const resolvedType = props.payload.type ?? (props.payload.count ? 'warning' : 'success');
    notification.create({
      title: props.payload.message,
      content,
      type: resolvedType,
      // Error notifications should stay until explicitly dismissed by the user.
      duration: resolvedType === 'error' ? 0 : 4000,
    });
  }
);
</script>

<template></template>
