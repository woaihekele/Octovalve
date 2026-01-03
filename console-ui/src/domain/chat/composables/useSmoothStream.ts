import { onBeforeUnmount } from 'vue';

type StreamDoneResolver = () => boolean;

export interface UseSmoothStreamOptions {
  onUpdate: (text: string) => void;
  streamDone: StreamDoneResolver;
  minDelay?: number;
  chunkFactor?: number;
  maxCharsPerFrame?: number;
  streamEndStrategy?: 'immediate' | 'progressive';
  initialText?: string;
}

const SUPPORTED_SEGMENTER_LOCALES = [
  'en-US',
  'de-DE',
  'es-ES',
  'zh-CN',
  'zh-TW',
  'ja-JP',
  'ru-RU',
  'el-GR',
  'fr-FR',
  'pt-PT',
] as const;

const createSegmenter = () => {
  if (typeof Intl === 'undefined' || typeof Intl.Segmenter === 'undefined') {
    return null;
  }
  try {
    return new Intl.Segmenter([...SUPPORTED_SEGMENTER_LOCALES]);
  } catch {
    return null;
  }
};

const segmenter = createSegmenter();

const segmentText = (text: string): string[] => {
  if (!text) return [];
  if (!segmenter) {
    return Array.from(text);
  }
  const segments: string[] = [];
  for (const part of segmenter.segment(text)) {
    segments.push(part.segment);
  }
  return segments;
};

type RafHandle = number | ReturnType<typeof setTimeout>;

export const useSmoothStream = (options: UseSmoothStreamOptions) => {
  const chunkQueue: string[] = [];
  let animationFrameId: RafHandle | null = null;
  let displayedText = options.initialText ?? '';
  let lastUpdateTime = 0;

  const {
    minDelay = 10,
    chunkFactor = 5,
    maxCharsPerFrame = Number.POSITIVE_INFINITY,
    streamEndStrategy = 'immediate',
  } = options;

  const getRaf = (): ((cb: FrameRequestCallback) => RafHandle) => {
    if (typeof window !== 'undefined' && typeof window.requestAnimationFrame === 'function') {
      return (cb: FrameRequestCallback) => window.requestAnimationFrame(cb);
    }
    if (typeof requestAnimationFrame === 'function') {
      return (cb: FrameRequestCallback) => requestAnimationFrame(cb);
    }
    return (cb: FrameRequestCallback) => setTimeout(() => cb(Date.now()), minDelay);
  };

  const getCancelRaf = (): ((handle: RafHandle) => void) => {
    if (typeof window !== 'undefined' && typeof window.cancelAnimationFrame === 'function') {
      return (handle: RafHandle) => window.cancelAnimationFrame(handle as number);
    }
    if (typeof cancelAnimationFrame === 'function') {
      return (handle: RafHandle) => cancelAnimationFrame(handle as number);
    }
    return (handle: RafHandle) => {
      clearTimeout(handle as ReturnType<typeof setTimeout>);
    };
  };

  const raf = getRaf();
  const cancelRaf = getCancelRaf();

  const scheduleNextFrame = () => {
    if (animationFrameId !== null) {
      return;
    }
    animationFrameId = raf(renderLoop);
  };

  const reset = (newText = '') => {
    if (animationFrameId !== null) {
      cancelRaf(animationFrameId);
      animationFrameId = null;
    }
    chunkQueue.length = 0;
    displayedText = newText;
    lastUpdateTime = 0;
    options.onUpdate(newText);
    scheduleNextFrame();
  };

  const addChunk = (chunk: string) => {
    if (!chunk) {
      return;
    }
    const chars = segmentText(chunk);
    if (chars.length === 0) {
      return;
    }
    chunkQueue.push(...chars);
    scheduleNextFrame();
  };

  const renderLoop = (currentTime: number) => {
    animationFrameId = null;

    if (chunkQueue.length === 0) {
      if (options.streamDone()) {
        options.onUpdate(displayedText);
        return;
      }
      scheduleNextFrame();
      return;
    }

    if (currentTime - lastUpdateTime < minDelay) {
      scheduleNextFrame();
      return;
    }
    lastUpdateTime = currentTime;

    const normalizedChunkFactor = Number.isFinite(chunkFactor) && chunkFactor > 0 ? chunkFactor : 5;
    let charsToRender = Math.max(1, Math.floor(chunkQueue.length / normalizedChunkFactor));
    if (Number.isFinite(maxCharsPerFrame) && maxCharsPerFrame > 0) {
      charsToRender = Math.min(charsToRender, Math.floor(maxCharsPerFrame));
    }
    if (options.streamDone() && streamEndStrategy === 'immediate') {
      charsToRender = chunkQueue.length;
    }

    const next = chunkQueue.splice(0, charsToRender).join('');
    displayedText += next;
    options.onUpdate(displayedText);

    if (chunkQueue.length > 0 || !options.streamDone()) {
      scheduleNextFrame();
    }
  };

  scheduleNextFrame();

  onBeforeUnmount(() => {
    if (animationFrameId !== null) {
      cancelRaf(animationFrameId);
      animationFrameId = null;
    }
    chunkQueue.length = 0;
  });

  return { addChunk, reset };
};
