import { invoke, isTauri } from '@tauri-apps/api/core';

const TAURI_AVAILABLE = isTauri();
const INTERCEPTOR_KEY = '__octovalveExternalLinkInterceptor';

type InterceptorState = {
  installed: boolean;
  uninstall: (() => void) | null;
};

function getInterceptorState(): InterceptorState {
  const global = globalThis as typeof globalThis & {
    [INTERCEPTOR_KEY]?: InterceptorState;
  };
  if (!global[INTERCEPTOR_KEY]) {
    global[INTERCEPTOR_KEY] = { installed: false, uninstall: null };
  }
  return global[INTERCEPTOR_KEY] as InterceptorState;
}

function isAllowedExternalUrl(url: string): boolean {
  const trimmed = url.trim();
  if (!trimmed) {
    return false;
  }
  return /^(https?:\/\/|mailto:|tel:)/i.test(trimmed);
}

export async function openExternalUrl(url: string): Promise<void> {
  const trimmed = url.trim();
  if (!trimmed) {
    return;
  }
  if (TAURI_AVAILABLE) {
    await invoke('open_external', { url: trimmed });
    return;
  }
  if (typeof window !== 'undefined') {
    window.open(trimmed, '_blank', 'noopener,noreferrer');
  }
}

export function ensureExternalLinkInterceptor(): void {
  if (typeof document === 'undefined') {
    return;
  }
  const state = getInterceptorState();
  if (state.installed) {
    return;
  }

  const handler = (event: MouseEvent) => {
    if (event.defaultPrevented) {
      return;
    }
    if (!(event.target instanceof Element)) {
      return;
    }
    const anchor = event.target.closest('a');
    if (!(anchor instanceof HTMLAnchorElement)) {
      return;
    }
    const href = anchor.getAttribute('href');
    if (!href || !isAllowedExternalUrl(href)) {
      return;
    }
    event.preventDefault();
    void openExternalUrl(href).catch((err) => {
      console.warn('[opener] failed to open external url:', err);
    });
  };

  document.addEventListener('click', handler, true);
  state.installed = true;
  state.uninstall = () => document.removeEventListener('click', handler, true);
}

