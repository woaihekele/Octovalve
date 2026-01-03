<script setup lang="ts">
import { watch } from 'vue';
import { useNotification } from 'naive-ui';

const props = defineProps<{
  payload: { message: string; count?: number } | null;
  token: number;
}>();

const notification = useNotification();

watch(
  () => props.token,
  () => {
    if (!props.payload) {
      return;
    }
    notification.create({
      title: props.payload.message,
      content: props.payload.count ? `${props.payload.count} 个待审批` : undefined,
      duration: 4000,
      type: props.payload.count ? 'warning' : 'success',
    });
  }
);
</script>

<template></template>
