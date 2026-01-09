import { appendReasoningBlock, concatAcpTextChunk, ensureToolCallBlock } from '../src/domain/chat/store/acpTimeline';

describe('acpTimeline', () => {
  it('concatAcpTextChunk avoids merging markdown strong markers across chunk boundaries', () => {
    expect(concatAcpTextChunk('**Title**', '**Next**')).toBe('**Title**\n**Next**');
    expect(concatAcpTextChunk('__Title__', '__Next__')).toBe('__Title__\n__Next__');
    expect(concatAcpTextChunk('Hello ', 'world')).toBe('Hello world');
    expect(concatAcpTextChunk('Hello', ' world')).toBe('Hello world');
  });

  it('splits reasoning blocks around tool calls', () => {
    let blocks: any[] | undefined = [];
    const idGen = (() => {
      let i = 0;
      return () => `r${(i += 1)}`;
    })();

    ({ blocks } = appendReasoningBlock(blocks, 'Think 1', idGen));
    ({ blocks } = ensureToolCallBlock(blocks, 'tool-1'));
    ({ blocks } = appendReasoningBlock(blocks, 'Think 2', idGen));
    ({ blocks } = ensureToolCallBlock(blocks, 'tool-2'));

    expect(blocks).toEqual([
      { type: 'reasoning', id: 'r1', content: 'Think 1' },
      { type: 'tool_call', toolCallId: 'tool-1' },
      { type: 'reasoning', id: 'r2', content: 'Think 2' },
      { type: 'tool_call', toolCallId: 'tool-2' },
    ]);
  });

  it('appends consecutive reasoning chunks into the same reasoning block', () => {
    let blocks: any[] | undefined = [];
    const idGen = () => 'r1';

    ({ blocks } = appendReasoningBlock(blocks, 'A', idGen));
    const result = appendReasoningBlock(blocks, 'B', idGen);
    blocks = result.blocks;

    expect(result.startedNewBlock).toBe(false);
    expect(blocks).toEqual([{ type: 'reasoning', id: 'r1', content: 'AB' }]);
  });

  it('does not duplicate tool call blocks', () => {
    let blocks: any[] | undefined = [{ type: 'tool_call', toolCallId: 'tool-1' }];
    const result = ensureToolCallBlock(blocks, 'tool-1');
    expect(result.inserted).toBe(false);
    expect(result.blocks).toBe(blocks);
  });
});

