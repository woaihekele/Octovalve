export type AppErrorPayload = {
  code: string;
  message: string;
  details?: unknown;
  retryable?: boolean;
};

export type NormalizedError = {
  code?: string;
  message: string;
  details?: unknown;
  retryable?: boolean;
  raw: unknown;
};

const ERROR_I18N_KEY: Record<string, string> = {
  CODEX_NOT_FOUND: 'errors.codexNotFound',
  CODEX_NOT_EXECUTABLE: 'errors.codexNotExecutable',
  CODEX_CONFIG_UNTRUSTED: 'errors.codexConfigUntrusted',
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

export function parseAppErrorPayload(raw: string): AppErrorPayload | null {
  const trimmed = raw.trim();
  if (!trimmed || (!trimmed.startsWith('{') && !trimmed.startsWith('['))) {
    return null;
  }
  try {
    const parsed = JSON.parse(trimmed) as unknown;
    if (!isRecord(parsed)) return null;
    const code = parsed.code;
    const message = parsed.message;
    if (typeof code !== 'string' || typeof message !== 'string') return null;
    return {
      code,
      message,
      details: parsed.details,
      retryable: typeof parsed.retryable === 'boolean' ? parsed.retryable : undefined,
    };
  } catch {
    return null;
  }
}

export function normalizeError(err: unknown): NormalizedError {
  // Tauri invoke errors、Error 对象、字符串等都会落到这里统一处理。
  if (typeof err === 'string') {
    const trimmed = err.trim();
    const payload = parseAppErrorPayload(trimmed);
    if (payload) {
      return { ...payload, raw: err };
    }
    // 兼容后端仅返回稳定错误码（例如 "CODEX_NOT_FOUND"）。
    if (trimmed && ERROR_I18N_KEY[trimmed]) {
      return { code: trimmed, message: trimmed, raw: err };
    }
    return { message: err, raw: err };
  }
  if (err instanceof Error) {
    const payload = parseAppErrorPayload(err.message);
    if (payload) {
      return { ...payload, raw: err };
    }
    const message = err.message || String(err);
    const trimmed = message.trim();
    if (trimmed && ERROR_I18N_KEY[trimmed]) {
      return { code: trimmed, message: trimmed, raw: err };
    }
    return { message, raw: err };
  }
  if (isRecord(err)) {
    // 兼容某些运行时可能直接抛对象：{ code, message, ... }
    const code = err.code;
    const message = err.message;
    if (typeof code === 'string' && typeof message === 'string') {
      return {
        code,
        message,
        details: err.details,
        retryable: typeof err.retryable === 'boolean' ? err.retryable : undefined,
        raw: err,
      };
    }
    // 兼容 { message: "..." } 形态
    if (typeof message === 'string') {
      const payload = parseAppErrorPayload(message);
      if (payload) {
        return { ...payload, raw: err };
      }
      return { message, raw: err };
    }
  }
  return { message: String(err ?? ''), raw: err };
}

export function formatErrorForUser(
  err: unknown,
  t: (key: string, params?: Record<string, unknown>) => string
): string {
  const normalized = normalizeError(err);
  if (normalized.code) {
    const key = ERROR_I18N_KEY[normalized.code];
    if (key) {
      // details 预留给将来做参数化展示
      return t(key);
    }
  }
  return normalized.message;
}
