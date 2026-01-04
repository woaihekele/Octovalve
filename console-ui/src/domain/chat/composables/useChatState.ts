import { ref, computed, watch } from 'vue';
import { i18n } from '../../../i18n';
import type { ChatMessage } from '../types';

export interface ChatInputState {
  inputValue: string;
  selectedImages: string[];
  selectedFiles: string[];
  isTextAreaFocused: boolean;
}

export function useChatState(messages: () => ChatMessage[]) {
  // Input state
  const inputValue = ref('');
  const selectedImages = ref<string[]>([]);
  const selectedFiles = ref<string[]>([]);
  const isTextAreaFocused = ref(false);

  // UI state
  const expandedRows = ref<Record<string, boolean>>({});
  const enableButtons = ref(false);
  const primaryButtonText = ref(i18n.global.t('chat.action.approve'));
  const secondaryButtonText = ref(i18n.global.t('chat.action.reject'));

  // Derived state
  const lastMessage = computed(() => messages().at(-1) ?? null);
  const secondLastMessage = computed(() => messages().at(-2) ?? null);

  const isAskingForApproval = computed(() => {
    const last = lastMessage.value;
    return last?.type === 'ask' && ['command', 'tool'].includes(last.ask ?? '');
  });

  const hasMessages = computed(() => messages().length > 0);

  // Actions
  function clearInput() {
    inputValue.value = '';
    selectedImages.value = [];
    selectedFiles.value = [];
  }

  function toggleRowExpanded(messageId: string) {
    expandedRows.value[messageId] = !expandedRows.value[messageId];
  }

  function isRowExpanded(messageId: string): boolean {
    return expandedRows.value[messageId] ?? false;
  }

  function setFocused(focused: boolean) {
    isTextAreaFocused.value = focused;
  }

  function addImage(imageUrl: string) {
    if (!selectedImages.value.includes(imageUrl)) {
      selectedImages.value.push(imageUrl);
    }
  }

  function removeImage(imageUrl: string) {
    const index = selectedImages.value.indexOf(imageUrl);
    if (index !== -1) {
      selectedImages.value.splice(index, 1);
    }
  }

  function addFile(filePath: string) {
    if (!selectedFiles.value.includes(filePath)) {
      selectedFiles.value.push(filePath);
    }
  }

  function removeFile(filePath: string) {
    const index = selectedFiles.value.indexOf(filePath);
    if (index !== -1) {
      selectedFiles.value.splice(index, 1);
    }
  }

  // Reset state when messages are cleared
  watch(
    () => messages().length,
    (newLen, oldLen) => {
      if (newLen === 0 && oldLen > 0) {
        expandedRows.value = {};
      }
    }
  );
  watch(
    () => i18n.global.locale.value,
    () => {
      primaryButtonText.value = i18n.global.t('chat.action.approve');
      secondaryButtonText.value = i18n.global.t('chat.action.reject');
    }
  );

  return {
    // Input state
    inputValue,
    selectedImages,
    selectedFiles,
    isTextAreaFocused,
    // UI state
    expandedRows,
    enableButtons,
    primaryButtonText,
    secondaryButtonText,
    // Derived
    lastMessage,
    secondLastMessage,
    isAskingForApproval,
    hasMessages,
    // Actions
    clearInput,
    toggleRowExpanded,
    isRowExpanded,
    setFocused,
    addImage,
    removeImage,
    addFile,
    removeFile,
  };
}
