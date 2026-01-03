<script setup lang="ts">
import { h, watch } from 'vue';
import { useNotification } from 'naive-ui';

const props = defineProps<{
  payload: { message: string; count?: number; target?: string } | null;
  token: number;
}>();

const emit = defineEmits<{
  (e: 'jump-pending', target: string): void;
}>();

const notification = useNotification();

watch(
  () => props.token,
  () => {
    if (!props.payload) {
      return;
    }
    const target = props.payload.target;
    const contentText = props.payload.count ? `${props.payload.count} 个待审批` : undefined;
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
    notification.create({
      title: props.payload.message,
      content,
      duration: 4000,
      type: props.payload.count ? 'warning' : 'success',
    });
  }
);
</script>

<template></template>
