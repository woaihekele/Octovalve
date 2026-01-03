<script setup lang="ts">
import { watch } from 'vue';
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
    notification.create({
      title: props.payload.message,
      content: props.payload.count ? `${props.payload.count} 个待审批` : undefined,
      duration: 4000,
      type: props.payload.count ? 'warning' : 'success',
      onClick: target ? () => emit('jump-pending', target) : undefined,
    });
  }
);
</script>

<template></template>
