import { ref, computed, onMounted, onUnmounted } from 'vue';
import { storeToRefs } from 'pinia';
import { useChatStore } from '../store/chatStore';
import type { SendMessageOptions, ChatMessage } from '../types';

export interface AcpConnection {
  send: (message: string) => void;
  close: () => void;
}

export function useChatService() {
  const store = useChatStore();
  const { messages, isStreaming, isConnected, error, canSend, activeSession } = storeToRefs(store);
  const abortController = ref<AbortController | null>(null);

  async function connect(): Promise<void> {
    store.setConnected(true);
    store.setError(null);
  }

  function disconnect(): void {
    if (abortController.value) {
      abortController.value.abort();
      abortController.value = null;
    }
    store.setConnected(false);
    store.setStreaming(false);
  }

  async function sendMessage(options: SendMessageOptions): Promise<void> {
    if (!store.canSend) {
      return;
    }

    // Add user message
    const userMessage = store.addMessage({
      type: 'say',
      say: 'text',
      role: 'user',
      content: options.content,
      status: 'complete',
      images: options.images,
      files: options.files,
    });

    // Create placeholder for assistant response
    const assistantMessage = store.addMessage({
      type: 'say',
      say: 'text',
      role: 'assistant',
      content: '',
      status: 'streaming',
      partial: true,
    });

    store.setStreaming(true);
    store.setError(null);

    try {
      abortController.value = new AbortController();

      // TODO: Replace with actual ACP API call
      // This is a placeholder that simulates streaming response
      await simulateStreamingResponse(assistantMessage.id, options.content);

      store.updateMessage(assistantMessage.id, {
        status: 'complete',
        partial: false,
      });
    } catch (error) {
      if (error instanceof Error && error.name === 'AbortError') {
        store.updateMessage(assistantMessage.id, {
          status: 'cancelled',
          partial: false,
        });
      } else {
        store.updateMessage(assistantMessage.id, {
          status: 'error',
          content: `错误: ${error instanceof Error ? error.message : '未知错误'}`,
          partial: false,
        });
        store.setError(error instanceof Error ? error.message : '发送消息失败');
      }
    } finally {
      store.setStreaming(false);
      abortController.value = null;
    }
  }

  async function simulateStreamingResponse(messageId: string, userInput: string): Promise<void> {
    // Placeholder: simulate streaming response
    const response = `收到你的消息: "${userInput}"\n\n这是一个模拟的 AI 响应。实际实现将连接到 ACP 后端。`;
    
    for (let i = 0; i < response.length; i++) {
      if (abortController.value?.signal.aborted) {
        throw new DOMException('Aborted', 'AbortError');
      }
      store.appendToMessage(messageId, response[i]);
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
  }

  function cancelCurrentRequest(): void {
    if (abortController.value) {
      abortController.value.abort();
    }
  }

  async function approveAction(messageId: string): Promise<void> {
    // TODO: Implement action approval via ACP
    console.log('Approve action:', messageId);
  }

  async function rejectAction(messageId: string): Promise<void> {
    // TODO: Implement action rejection via ACP
    console.log('Reject action:', messageId);
  }

  onMounted(() => {
    connect();
  });

  onUnmounted(() => {
    disconnect();
  });

  return {
    // State from store (reactive refs)
    messages,
    isStreaming,
    isConnected,
    error,
    canSend,
    activeSession,
    // Actions
    sendMessage,
    cancelCurrentRequest,
    approveAction,
    rejectAction,
    connect,
    disconnect,
    createSession: store.createSession,
    clearMessages: store.clearMessages,
  };
}
