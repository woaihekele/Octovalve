<template>
  <n-drawer
    v-model:show="isOpen"
    :width="420"
    placement="right"
    :trap-focus="false"
    :block-scroll="false"
    :close-on-esc="true"
  >
    <n-drawer-content :native-scrollbar="false" body-content-style="padding: 0;">
      <ChatView :title="title" :greeting="greeting" />
    </n-drawer-content>
  </n-drawer>
</template>

<script setup lang="ts">
import { computed } from 'vue';
import { NDrawer, NDrawerContent } from 'naive-ui';
import ChatView from './ChatView.vue';

interface Props {
  modelValue: boolean;
  title?: string;
  greeting?: string;
}

const props = withDefaults(defineProps<Props>(), {
  title: 'AI 助手',
  greeting: '你好，我是 AI 助手',
});

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
}>();

const isOpen = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value),
});
</script>
