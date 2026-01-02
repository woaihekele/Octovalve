import { Marked, type Tokens } from 'marked';
import DOMPurify from 'dompurify';
import hljs from 'highlight.js/lib/common';

const md = new Marked({
  gfm: true,
  breaks: true,
});

const escapeHtml = (value: string): string =>
  value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');

const normalizeLanguage = (value?: string | null): string => {
  if (!value) return '';
  return value.trim().split(/\s+/, 1)[0].toLowerCase();
};

md.use({
  renderer: {
    code(token: Tokens.Code) {
      const source = token.text ?? '';
      const explicitLanguage = normalizeLanguage(token.lang);
      try {
        const highlightResult =
          explicitLanguage && hljs.getLanguage(explicitLanguage)
            ? hljs.highlight(source, { language: explicitLanguage, ignoreIllegals: true })
            : hljs.highlightAuto(source);
        const resolvedLanguage = normalizeLanguage(highlightResult.language || explicitLanguage) || 'plaintext';
        return `<pre><code class="hljs language-${resolvedLanguage}">${highlightResult.value}</code></pre>\n`;
      } catch {
        return `<pre><code class="hljs">${escapeHtml(source)}</code></pre>\n`;
      }
    },
  },
});

export function renderSafeMarkdown(input: string): string {
  try {
    const src = input ?? '';
    const html = md.parse(src, { async: false }) as string;
    return DOMPurify.sanitize(html);
  } catch {
    const text = input ?? '';
    return escapeHtml(text);
  }
}
