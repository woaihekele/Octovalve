import { nextTick, onBeforeUnmount, ref, type Ref } from 'vue';

export interface StickToBottomOptions {
  force?: boolean;
  behavior?: ScrollBehavior;
}

const BOTTOM_THRESHOLD = 80;
const DIRECTION_EPS = 2;
const SMOOTH_DISTANCE_LIMIT = 1200;

export function useStickToBottom(
  containerRef: Ref<HTMLElement | null>,
  contentRef?: Ref<HTMLElement | null>
) {
  const stickToBottom = ref(true);

  let currentContainer: HTMLElement | null = null;
  let currentContent: HTMLElement | null = null;

  let isProgrammaticScroll = false;
  let lastScrollTop = 0;
  let lastScrollHeight = 0;
  let lastClientHeight = 0;

  let resizeObserver: ResizeObserver | null = null;
  let mutationObserver: MutationObserver | null = null;
  let detachUserIntentListeners: (() => void) | null = null;

  let touchStartY: number | null = null;
  let hasUserScrollIntent = false;

  let lastContentHeight = 0;

  let scrollAnimFrame: number | null = null;
  let scrollAnimToken = 0;

  let throttleTimer: number | null = null;
  let lastThrottleAt = 0;
  const throttleMs = 200;

  const computeDistance = (element: HTMLElement) => {
    return Math.max(element.scrollHeight - element.scrollTop - element.clientHeight, 0);
  };

  const stopScrollAnimation = () => {
    if (scrollAnimFrame !== null) {
      scrollAnimToken += 1;
      cancelAnimationFrame(scrollAnimFrame);
      scrollAnimFrame = null;
    }
  };

  const interruptStickToBottom = () => {
    if (!stickToBottom.value && !isProgrammaticScroll) {
      return;
    }
    stickToBottom.value = false;
    isProgrammaticScroll = false;
    stopScrollAnimation();
  };

  const bindUserIntentListeners = (element: HTMLElement | null) => {
    if (currentContainer === element) {
      return;
    }
    if (detachUserIntentListeners) {
      detachUserIntentListeners();
      detachUserIntentListeners = null;
    }
    currentContainer = element;
    if (!element) {
      return;
    }

    lastScrollTop = element.scrollTop;
    lastScrollHeight = element.scrollHeight;
    lastClientHeight = element.clientHeight;

    const handleWheel = (event: WheelEvent) => {
      if (event.deltaY < 0) {
        hasUserScrollIntent = true;
        interruptStickToBottom();
      }
    };

    const handleTouchStart = (event: TouchEvent) => {
      touchStartY = event.touches[0]?.clientY ?? null;
    };

    const handleTouchMove = (event: TouchEvent) => {
      if (touchStartY == null) {
        return;
      }
      const currentY = event.touches[0]?.clientY;
      if (typeof currentY !== 'number') {
        return;
      }
      const delta = currentY - touchStartY;
      if (delta > 4) {
        hasUserScrollIntent = true;
        interruptStickToBottom();
        touchStartY = currentY;
      }
    };

    element.addEventListener('wheel', handleWheel, { passive: true });
    element.addEventListener('touchstart', handleTouchStart, { passive: true });
    element.addEventListener('touchmove', handleTouchMove, { passive: true });

    detachUserIntentListeners = () => {
      element.removeEventListener('wheel', handleWheel);
      element.removeEventListener('touchstart', handleTouchStart);
      element.removeEventListener('touchmove', handleTouchMove);
      touchStartY = null;
    };
  };

  const bindContentObserver = (element: HTMLElement | null) => {
    if (currentContent === element) {
      return;
    }
    if (resizeObserver) {
      resizeObserver.disconnect();
      resizeObserver = null;
    }
    if (mutationObserver) {
      mutationObserver.disconnect();
      mutationObserver = null;
    }
    currentContent = element;
    lastContentHeight = element?.scrollHeight ?? 0;

    if (!element) {
      return;
    }

    const handleGrowth = () => {
      const nextHeight = element.scrollHeight;
      const delta = nextHeight - lastContentHeight;
      lastContentHeight = nextHeight;
      if (delta > 0) {
        scheduleStickyScroll();
      }
    };

    if (typeof ResizeObserver !== 'undefined') {
      resizeObserver = new ResizeObserver(() => {
        handleGrowth();
      });
      resizeObserver.observe(element);
    }

    mutationObserver = new MutationObserver(() => {
      handleGrowth();
    });
    mutationObserver.observe(element, { childList: true, subtree: true, characterData: true });
  };

  const ensureBindings = () => {
    const container = containerRef.value;
    if (!container) {
      return null;
    }
    bindUserIntentListeners(container);
    bindContentObserver(contentRef?.value ?? container);
    return container;
  };

  const resolveBehavior = (options: StickToBottomOptions, distance: number, force: boolean): ScrollBehavior => {
    if (options.behavior) {
      return options.behavior;
    }
    if (force) {
      return 'auto';
    }
    return distance <= SMOOTH_DISTANCE_LIMIT ? 'smooth' : 'auto';
  };

  const scrollToBottom = async (options: StickToBottomOptions = {}) => {
    await nextTick();

    const force = Boolean(options.force);
    if (!force && !stickToBottom.value) {
      return;
    }

    const element = ensureBindings();
    if (!element) {
      return;
    }

    const distance = computeDistance(element);
    const behavior = resolveBehavior(options, distance, force);

    const targetTop = Math.max(element.scrollHeight - element.clientHeight, 0);
    const startTop = element.scrollTop;
    const delta = targetTop - startTop;
    const absDelta = Math.abs(delta);

    if (absDelta <= 1) {
      element.scrollTop = targetTop;
      isProgrammaticScroll = false;
      if (force) {
        stickToBottom.value = true;
      }
      return;
    }

    stopScrollAnimation();

    if (behavior === 'smooth') {
      isProgrammaticScroll = true;
      const duration = Math.min(1400, Math.max(600, absDelta * 1.3));
      const token = scrollAnimToken + 1;
      scrollAnimToken = token;
      const startTime = performance.now();
      const easeOutQuart = (t: number) => 1 - Math.pow(1 - t, 4);

      const step = (now: number) => {
        if (scrollAnimToken !== token) {
          scrollAnimFrame = null;
          return;
        }
        const progress = Math.min(1, (now - startTime) / duration);
        const eased = easeOutQuart(progress);
        element.scrollTop = startTop + delta * eased;
        if (progress < 1) {
          scrollAnimFrame = requestAnimationFrame(step);
        } else {
          scrollAnimFrame = null;
          isProgrammaticScroll = false;
        }
      };

      scrollAnimFrame = requestAnimationFrame(step);
    } else {
      isProgrammaticScroll = true;
      element.scrollTo({ top: Number.MAX_SAFE_INTEGER, behavior: 'auto' });
      isProgrammaticScroll = false;
    }

    if (force) {
      stickToBottom.value = true;
    }
  };

  const scheduleStickyScroll = () => {
    if (!stickToBottom.value) {
      return;
    }
    if (throttleTimer !== null) {
      return;
    }

    const now = Date.now();
    const elapsed = now - lastThrottleAt;
    const run = () => {
      throttleTimer = null;
      lastThrottleAt = Date.now();
      void scrollToBottom({ force: false });
    };

    if (elapsed >= throttleMs) {
      run();
    } else {
      throttleTimer = window.setTimeout(run, throttleMs - elapsed);
    }
  };

  const handleScroll = () => {
    const element = containerRef.value;
    if (!element) {
      return;
    }

    ensureBindings();

    const userIntent = hasUserScrollIntent;
    hasUserScrollIntent = false;

    const distance = computeDistance(element);
    const containerSizeChanged =
      element.scrollHeight !== lastScrollHeight || element.clientHeight !== lastClientHeight;

    const scrollingUp = element.scrollTop < lastScrollTop - DIRECTION_EPS;
    const scrollingDown = element.scrollTop > lastScrollTop + DIRECTION_EPS;

    lastScrollTop = element.scrollTop;
    lastScrollHeight = element.scrollHeight;
    lastClientHeight = element.clientHeight;

    if (isProgrammaticScroll) {
      if (scrollingUp && userIntent) {
        isProgrammaticScroll = false;
        stickToBottom.value = false;
        stopScrollAnimation();
        return;
      }

      if (distance < BOTTOM_THRESHOLD) {
        stickToBottom.value = true;
        isProgrammaticScroll = false;
      }
      return;
    }

    if (scrollingUp) {
      if (containerSizeChanged && stickToBottom.value) {
        return;
      }
      if (stickToBottom.value && distance < BOTTOM_THRESHOLD) {
        return;
      }
      if (!userIntent) {
        return;
      }
      stickToBottom.value = false;
      return;
    }

    if (distance < BOTTOM_THRESHOLD) {
      stickToBottom.value = true;
      return;
    }

    if (scrollingDown && !stickToBottom.value && distance < BOTTOM_THRESHOLD) {
      stickToBottom.value = true;
    }
  };

  const activateStickToBottom = () => {
    stickToBottom.value = true;
  };

  onBeforeUnmount(() => {
    if (detachUserIntentListeners) {
      detachUserIntentListeners();
      detachUserIntentListeners = null;
    }
    currentContainer = null;
    if (resizeObserver) {
      resizeObserver.disconnect();
      resizeObserver = null;
    }
    if (mutationObserver) {
      mutationObserver.disconnect();
      mutationObserver = null;
    }
    currentContent = null;
    if (throttleTimer !== null) {
      clearTimeout(throttleTimer);
      throttleTimer = null;
    }
    stopScrollAnimation();
  });

  return {
    stickToBottom,
    scrollToBottom,
    handleScroll,
    activateStickToBottom,
  };
}
