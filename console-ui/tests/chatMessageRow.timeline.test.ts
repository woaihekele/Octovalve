import { mount } from '@vue/test-utils';
import { defineComponent, type PropType } from 'vue';
import ChatMessageRow from '../src/domain/chat/components/ChatMessageRow.vue';

const ReasoningBlockStub = defineComponent({
  name: 'ReasoningBlock',
  props: {
    previewText: { type: String, default: '' },
  },
  template: `<div data-kind="reasoning" :data-content="previewText"><slot name="body" /></div>`,
});

const ToolCallCardStub = defineComponent({
  name: 'ToolCallCard',
  props: {
    toolCall: { type: Object as PropType<{ id: string }>, required: true },
  },
  template: `<div data-kind="tool" :data-id="toolCall.id"></div>`,
});

const ChatMarkdownStub = defineComponent({
  name: 'ChatMarkdown',
  props: {
    text: { type: String, default: '' },
  },
  template: `<div data-kind="markdown">{{ text }}</div>`,
});

describe('ChatMessageRow timeline blocks', () => {
  it('renders reasoning/tool blocks in order', () => {
    const message: any = {
      id: 'm1',
      ts: 0,
      type: 'say',
      say: 'text',
      role: 'assistant',
      content: '',
      status: 'streaming',
      partial: true,
      toolCalls: [
        { id: 'tool-1', name: 'bash', arguments: {}, status: 'completed' },
        { id: 'tool-2', name: 'bash', arguments: {}, status: 'completed' },
      ],
      blocks: [
        { type: 'reasoning', id: 'r1', content: 'Think 1' },
        { type: 'tool_call', toolCallId: 'tool-1' },
        { type: 'reasoning', id: 'r2', content: 'Think 2' },
        { type: 'tool_call', toolCallId: 'tool-2' },
      ],
    };

    const wrapper = mount(ChatMessageRow, {
      props: { message, isLast: true },
      global: {
        stubs: {
          ReasoningBlock: ReasoningBlockStub,
          ToolCallCard: ToolCallCardStub,
          ChatMarkdown: ChatMarkdownStub,
        },
        mocks: {
          $t: (key: string) => key,
        },
      },
    });

    const sequence = Array.from(wrapper.element.querySelectorAll('[data-kind]'))
      .filter((el) => {
        const kind = el.getAttribute('data-kind');
        return kind === 'reasoning' || kind === 'tool';
      })
      .map((el) => {
        const kind = el.getAttribute('data-kind');
        if (kind === 'reasoning') {
          return `reasoning:${el.getAttribute('data-content')}`;
        }
        return `tool:${el.getAttribute('data-id')}`;
      });

    expect(sequence).toEqual(['reasoning:Think 1', 'tool:tool-1', 'reasoning:Think 2', 'tool:tool-2']);
  });
});
