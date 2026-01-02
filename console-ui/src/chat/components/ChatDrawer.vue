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
      <ChatView
        :is-open="isOpen"
        :title="title"
        :greeting="greeting"
        :messages="messages"
        :is-streaming="isStreaming"
        :is-connected="isConnected"
        :provider="provider"
        @send="$emit('send', $event)"
        @cancel="$emit('cancel')"
        @new-session="$emit('new-session')"
        @clear="$emit('clear')"
        @change-provider="$emit('change-provider', $event)"
      />
    </n-drawer-content>
  </n-drawer>
</template>

<script setup lang="ts">
import { computed } from 'vue';
import { NDrawer, NDrawerContent } from 'naive-ui';
import ChatView from './ChatView.vue';
import type { ChatMessage } from '../types';

interface Props {
  modelValue: boolean;
  title?: string;
  greeting?: string;
  messages: ChatMessage[];
  isStreaming?: boolean;
  isConnected?: boolean;
  provider?: 'acp' | 'openai';
}

const props = withDefaults(defineProps<Props>(), {
  title: 'AI 助手',
  greeting: '你好，我是 AI 助手',
  isStreaming: false,
  isConnected: true,
  provider: 'acp',
});

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
  send: [content: string];
  cancel: [];
  'new-session': [];
  clear: [];
  'change-provider': [provider: 'acp' | 'openai'];
}>();

const isOpen = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value),
});
</script>
