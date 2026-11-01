import type { FileKind } from './bindings/FileKind';
import type { ClassKind } from './bindings/ClassKind';
import type { ProjectModel } from './bindings/ProjectModel';

export const KIND_LETTER: Record<string, string> = {
  module: 'M',
  controller: 'C',
  service: 'S',
  provider: 'P',
  entity: 'E',
  schema: 'E',
  dto: 'D',
  spec: 'T',
  guard: 'G',
  pipe: 'P',
  interceptor: 'I',
  filter: 'F',
  middleware: 'W',
  gateway: 'W',
  resolver: 'R',
  strategy: 'S',
  main: '>',
  other: '·',
};

export function kindOf(path: string): FileKind {
  const name = path.split('/').pop() ?? path;
  if (/\.(spec|e2e-spec|test)\.ts$/.test(name)) return 'spec';
  const table: [string, FileKind][] = [
    ['.module.ts', 'module'],
    ['.controller.ts', 'controller'],
    ['.service.ts', 'service'],
    ['.entity.ts', 'entity'],
    ['.schema.ts', 'entity'],
    ['.model.ts', 'entity'],
    ['.dto.ts', 'dto'],
    ['.input.ts', 'dto'],
    ['.guard.ts', 'guard'],
    ['.pipe.ts', 'pipe'],
    ['.interceptor.ts', 'interceptor'],
    ['.filter.ts', 'filter'],
    ['.middleware.ts', 'middleware'],
    ['.gateway.ts', 'gateway'],
    ['.resolver.ts', 'resolver'],
  ];
  for (const [s, k] of table) if (name.endsWith(s)) return k;
  if (name === 'main.ts') return 'main';
  return 'other';
}

export function basename(p: string) {
  return p.split('/').pop() ?? p;
}

export function dirname(p: string) {
  const i = p.lastIndexOf('/');
  return i < 0 ? '' : p.slice(0, i);
}

export function classKindLabel(k: ClassKind) {
  return k.charAt(0).toUpperCase() + k.slice(1);
}

export function languageFor(path: string) {
  if (path.endsWith('.ts')) return 'typescript';
  if (path.endsWith('.js') || path.endsWith('.mjs') || path.endsWith('.cjs')) return 'javascript';
  if (path.endsWith('.json')) return 'json';
  if (path.endsWith('.md')) return 'markdown';
  if (path.endsWith('.yml') || path.endsWith('.yaml')) return 'yaml';
  if (path.endsWith('.prisma')) return 'prisma';
  if (path.endsWith('.graphql') || path.endsWith('.gql')) return 'graphql';
  if (path.endsWith('.html')) return 'html';
  if (path.endsWith('.css')) return 'css';
  if (basename(path).startsWith('.env')) return 'ini';
  if (basename(path) === 'Dockerfile') return 'dockerfile';
  return 'plaintext';
}

/** The module that owns a file (by folder), for "Generate in X" and the breadcrumb. */
export function moduleForFile(model: ProjectModel | null, path: string | null) {
  if (!model || !path) return null;
  const info = model.files.find((f) => f.path === path);
  const name = info?.module ?? model.rootModules[0] ?? null;
  return name ? (model.modules.find((m) => m.name === name) ?? null) : null;
}

// ---- ANSI → HTML ----------------------------------------------------------------------
const ANSI_COLORS: Record<number, string> = {
  30: 'var(--text-3)',
  31: 'var(--error)',
  32: 'var(--success)',
  33: 'var(--warning)',
  34: 'var(--info)',
  35: 'var(--c-dto)',
  36: 'var(--info)',
  37: 'var(--text)',
  90: 'var(--text-3)',
  91: 'var(--error)',
  92: 'var(--success)',
  93: 'var(--warning)',
  94: 'var(--info)',
  95: 'var(--c-dto)',
  96: 'var(--info)',
  97: 'var(--text)',
};

function escapeHtml(s: string) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

export function ansiToHtml(input: string): string {
  let out = '';
  let open = false;
  let color: string | null = null;
  let bold = false;
  const re = /\x1b\[([0-9;]*)m/g;
  let last = 0;
  let m: RegExpExecArray | null;
  const text = input.replace(/\x1b\[[0-9;]*[A-HJKSTfnsu]/g, '').replace(/\x1b\][^\x07]*\x07/g, '');
  const flush = (chunk: string) => {
    if (!chunk) return;
    out += escapeHtml(chunk);
  };
  while ((m = re.exec(text))) {
    flush(text.slice(last, m.index));
    last = re.lastIndex;
    const codes = m[1].split(';').filter(Boolean).map(Number);
    if (codes.length === 0) codes.push(0);
    for (const c of codes) {
      if (c === 0) {
        color = null;
        bold = false;
      } else if (c === 1) bold = true;
      else if (c === 22) bold = false;
      else if (c === 39) color = null;
      else if (ANSI_COLORS[c]) color = ANSI_COLORS[c];
    }
    if (open) out += '</span>';
    open = !!(color || bold);
    if (open) out += `<span style="${color ? `color:${color};` : ''}${bold ? 'font-weight:600;' : ''}">`;
  }
  flush(text.slice(last));
  if (open) out += '</span>';
  return out;
}

export function stripAnsi(s: string) {
  return s.replace(/\x1b\[[0-9;]*[A-Za-z]/g, '');
}

export function fuzzyScore(query: string, text: string): number {
  if (!query) return 1;
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  const idx = t.indexOf(q);
  if (idx >= 0) return 1000 - idx - t.length / 100;
  let ti = 0;
  let score = 0;
  for (const ch of q) {
    const found = t.indexOf(ch, ti);
    if (found < 0) return -1;
    score += found === ti ? 3 : 1;
    ti = found + 1;
  }
  return score - t.length / 100;
}

export function timeAgo(ms: number) {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  return `${Math.round(m / 60)}h ago`;
}
