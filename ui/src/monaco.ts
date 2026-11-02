// Monaco setup: workers, Forge themes, decorator highlighting, Nest-aware code lenses,
// definitions and diagnostics from the Rust model, and the buffer (model) registry.
import * as monaco from 'monaco-editor';
import EditorWorker from 'monaco-editor/editor/editor.worker?worker';
import TsWorker from 'monaco-editor/languages/features/typescript/ts.worker?worker';
import JsonWorker from 'monaco-editor/languages/features/json/json.worker?worker';
import { language as tsLanguage } from 'monaco-editor/languages/definitions/typescript/typescript';
import { api } from './api';
import { useApp, openFile, setDirty, toast, errorText, scheduleTypecheck } from './store';
import { languageFor } from './util';
import type { ProjectModel } from './bindings/ProjectModel';

(self as unknown as { MonacoEnvironment: monaco.Environment }).MonacoEnvironment = {
  getWorker(_id: string, label: string) {
    if (label === 'typescript' || label === 'javascript') return new TsWorker();
    if (label === 'json') return new JsonWorker();
    return new EditorWorker();
  },
};

export { monaco };

// ---- themes -------------------------------------------------------------------------------
monaco.editor.defineTheme('forge-dark', {
  base: 'vs-dark',
  inherit: true,
  rules: [
    { token: '', foreground: 'ECEAE6' },
    { token: 'keyword', foreground: 'C9A7FF' },
    { token: 'decorator', foreground: 'F2B44C' },
    { token: 'string', foreground: '9AD49A' },
    { token: 'string.escape', foreground: '9AD49A' },
    { token: 'type.identifier', foreground: '6CC9F0' },
    { token: 'number', foreground: '6CC9F0' },
    { token: 'comment', foreground: '878A92', fontStyle: 'italic' },
    { token: 'regexp', foreground: 'E6B8A2' },
    { token: 'delimiter', foreground: 'A3A6AD' },
    { token: 'key', foreground: '6CC9F0' },
    { token: 'string.key.json', foreground: '6CC9F0' },
    { token: 'string.value.json', foreground: '9AD49A' },
  ],
  colors: {
    'editor.background': '#191A1E',
    'editor.foreground': '#ECEAE6',
    'editorLineNumber.foreground': '#82858D',
    'editorLineNumber.activeForeground': '#ECEAE6',
    'editor.lineHighlightBackground': '#22232A',
    'editor.lineHighlightBorder': '#00000000',
    'editor.selectionBackground': '#3B3D45',
    'editor.inactiveSelectionBackground': '#2C2E34',
    'editorCursor.foreground': '#F2675F',
    'editorIndentGuide.background1': '#26282D',
    'editorIndentGuide.activeBackground1': '#3A3C42',
    'editorWidget.background': '#141518',
    'editorWidget.border': '#2A2C31',
    'editorSuggestWidget.background': '#141518',
    'editorSuggestWidget.selectedBackground': '#26282D',
    'editorHoverWidget.background': '#141518',
    'editorCodeLens.foreground': '#A3A6AD',
    'editorGutter.background': '#191A1E',
    'diffEditor.insertedLineBackground': '#10291F',
    'diffEditor.insertedTextBackground': '#1B4A3666',
    'diffEditor.removedLineBackground': '#2A1C1C',
    'diffEditor.removedTextBackground': '#5A272466',
    'scrollbarSlider.background': '#2A2C3199',
    'scrollbarSlider.hoverBackground': '#3A3C42',
    'editorError.foreground': '#F2675F',
    'editorWarning.foreground': '#F2B44C',
    'editorInfo.foreground': '#6CC9F0',
    'minimap.background': '#191A1E',
  },
});

monaco.editor.defineTheme('forge-light', {
  base: 'vs',
  inherit: true,
  rules: [
    { token: '', foreground: '1A1B1E' },
    { token: 'keyword', foreground: '6F45C8' },
    { token: 'decorator', foreground: '8A5700' },
    { token: 'string', foreground: '3C7A2E' },
    { token: 'type.identifier', foreground: '1B65A0' },
    { token: 'number', foreground: '1B65A0' },
    { token: 'comment', foreground: '6B6E75', fontStyle: 'italic' },
    { token: 'regexp', foreground: '9C4F2E' },
    { token: 'delimiter', foreground: '50535A' },
    { token: 'string.key.json', foreground: '1B65A0' },
    { token: 'string.value.json', foreground: '3C7A2E' },
  ],
  colors: {
    'editor.background': '#FDFCFA',
    'editor.foreground': '#1A1B1E',
    'editorLineNumber.foreground': '#6E7177',
    'editorLineNumber.activeForeground': '#1A1B1E',
    'editor.lineHighlightBackground': '#F1EFEA',
    'editor.lineHighlightBorder': '#00000000',
    'editorCursor.foreground': '#B8372F',
    'editorWidget.background': '#F4F3EF',
    'editorWidget.border': '#D3D0C8',
    'editorGutter.background': '#FDFCFA',
    'diffEditor.insertedLineBackground': '#E2F3E6',
    'diffEditor.removedLineBackground': '#F8DEDB',
    'editorCodeLens.foreground': '#50535A',
  },
});

export function applyTheme(theme: string) {
  const dark = theme === 'dark' || (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  monaco.editor.setTheme(dark ? 'forge-dark' : 'forge-light');
}

// ---- TypeScript language ---------------------------------------------------------------------
const ts = monaco.typescript;
ts.typescriptDefaults.setCompilerOptions({
  target: ts.ScriptTarget.ES2020,
  module: ts.ModuleKind.CommonJS,
  moduleResolution: ts.ModuleResolutionKind.NodeJs,
  experimentalDecorators: true,
  emitDecoratorMetadata: true,
  allowNonTsExtensions: true,
  strict: false,
});
// Imports from node_modules can't be resolved in the browser worker; real type errors come
// from the project's own TypeScript compiler (Run › Type check, and on save).
ts.typescriptDefaults.setDiagnosticsOptions({ noSemanticValidation: true, noSyntaxValidation: false });

// Decorators get their own token so `@Controller` reads as Nest metadata.
function applyMonarch() {
  const lang = structuredClone(tsLanguage) as monaco.languages.IMonarchLanguage & { tokenizer: Record<string, unknown[]> };
  lang.tokenizer.common = [[/@[a-zA-Z_$][\w$]*/, 'decorator'], ...(lang.tokenizer.common ?? [])];
  monaco.languages.setMonarchTokensProvider('typescript', lang);
}
applyMonarch();
monaco.languages.onLanguageEncountered('typescript', () => {
  // The built-in loader sets the default grammar asynchronously; re-apply ours after it.
  for (const delay of [0, 50, 250, 1000]) setTimeout(applyMonarch, delay);
});

// ---- buffers ---------------------------------------------------------------------------------
const saved = new Map<string, number>();
const loading = new Map<string, Promise<monaco.editor.ITextModel>>();

function root() {
  return useApp.getState().model?.root ?? '';
}

export function uriFor(path: string) {
  return monaco.Uri.file(`${root()}/${path}`);
}

export function pathForUri(uri: monaco.Uri) {
  const r = root().replace(/\\/g, '/').toLowerCase();
  const p = uri.path.replace(/^\/([a-zA-Z]:)/, '$1');
  return p.toLowerCase().startsWith(`${r}/`) ? p.slice(r.length + 1) : null;
}

export function getModel(path: string): Promise<monaco.editor.ITextModel> {
  const existing = monaco.editor.getModel(uriFor(path));
  if (existing) return Promise.resolve(existing);
  if (loading.has(path)) return loading.get(path)!;
  const p = api.readFile(path).then((content) => {
    const again = monaco.editor.getModel(uriFor(path));
    if (again) return again;
    const m = monaco.editor.createModel(content, languageFor(path), uriFor(path));
    saved.set(path, m.getAlternativeVersionId());
    m.onDidChangeContent(() => setDirty(path, m.getAlternativeVersionId() !== saved.get(path)));
    return m;
  });
  loading.set(path, p);
  p.finally(() => loading.delete(path));
  return p;
}

export function isDirty(path: string) {
  const m = monaco.editor.getModel(uriFor(path));
  return !!m && m.getAlternativeVersionId() !== saved.get(path);
}

// ---- formatting --------------------------------------------------------------------------------
// The project's Prettier formats every language it knows, like the VS Code extension. Without
// Prettier, TypeScript and JavaScript fall back to the TypeScript formatter and JSON to a plain
// re-indent. The built-in document formatters for JSON, CSS and HTML are turned off so this is
// the one Format Document (Shift Alt F) uses; Format Selection keeps the built-in ones.
const FORMAT_LANGUAGES = ['typescript', 'javascript', 'json', 'css', 'scss', 'less', 'html', 'markdown', 'yaml', 'graphql'];

for (const d of [monaco.json.jsonDefaults, monaco.css.cssDefaults, monaco.css.scssDefaults, monaco.css.lessDefaults, monaco.html.htmlDefaults]) {
  d.setModeConfiguration({ ...d.modeConfiguration, documentFormattingEdits: false });
}

function setFormatter(name: string, version: string | null, error: string | null) {
  const cur = useApp.getState().formatter;
  if (cur?.name !== name || cur.version !== version || cur.error !== error) useApp.setState({ formatter: { name, version, error } });
}

/** TypeScript's own formatter, with the options Monaco's TypeScript support uses. */
async function builtinFormat(model: monaco.editor.ITextModel, opts: monaco.languages.FormattingOptions): Promise<string | null> {
  const lang = model.getLanguageId();
  if (lang === 'json') {
    try {
      return JSON.stringify(JSON.parse(model.getValue()), null, opts.insertSpaces ? opts.tabSize : '\t') + '\n';
    } catch {
      return null; // JSON with comments, or broken: leave it.
    }
  }
  if (lang !== 'typescript' && lang !== 'javascript') return null;
  const getWorker = lang === 'typescript' ? monaco.typescript.getTypeScriptWorker : monaco.typescript.getJavaScriptWorker;
  const worker = await (await getWorker())(model.uri);
  const changes: { span: { start: number; length: number }; newText: string }[] = await worker.getFormattingEditsForDocument(model.uri.toString(), {
    ConvertTabsToSpaces: opts.insertSpaces,
    TabSize: opts.tabSize,
    IndentSize: opts.tabSize,
    IndentStyle: 2,
    NewLineCharacter: '\n',
    InsertSpaceAfterCommaDelimiter: true,
    InsertSpaceAfterSemicolonInForStatements: true,
    InsertSpaceBeforeAndAfterBinaryOperators: true,
    InsertSpaceAfterKeywordsInControlFlowStatements: true,
    InsertSpaceAfterFunctionKeywordForAnonymousFunctions: true,
    InsertSpaceAfterOpeningAndBeforeClosingNonemptyParenthesis: false,
    InsertSpaceAfterOpeningAndBeforeClosingNonemptyBrackets: false,
    InsertSpaceAfterOpeningAndBeforeClosingTemplateStringBraces: false,
    PlaceOpenBraceOnNewLineForControlBlocks: false,
    PlaceOpenBraceOnNewLineForFunctions: false,
  });
  let text = model.getValue();
  for (const c of [...changes].sort((a, b) => b.span.start - a.span.start)) text = text.slice(0, c.span.start) + c.newText + text.slice(c.span.start + c.span.length);
  return text;
}

/** The formatted text of a buffer, or null when there's nothing to do (or it couldn't be formatted). */
async function formattedText(model: monaco.editor.ITextModel, opts: monaco.languages.FormattingOptions): Promise<string | null> {
  const path = pathForUri(model.uri);
  if (!path) return null;
  try {
    const r = await api.formatFile(path, model.getValue(monaco.editor.EndOfLinePreference.LF));
    if (r.formatter) {
      setFormatter('Prettier', r.version, r.error);
      return r.error ? null : r.text;
    }
  } catch (e) {
    setFormatter('Prettier', null, errorText(e));
    return null;
  }
  try {
    const text = await builtinFormat(model, opts);
    if (text !== null) setFormatter('Built-in', null, null);
    return text;
  } catch (e) {
    setFormatter('Built-in', null, errorText(e));
    return null;
  }
}

/** Indices [a1, a2) of `a` to replace with [b1, b2) of `b`, from a line LCS of the changed middle. */
function lineHunks(a: string[], b: string[]): [number, number, number, number][] {
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const n = endA - start;
  const m = endB - start;
  if (n === 0 && m === 0) return [];
  // Past ~4M cells the table costs more than it saves; replace the middle in one piece.
  if (n * m > 4_000_000) return [[start, endA, start, endB]];
  const w = m + 1;
  const lcs = new Uint16Array((n + 1) * w);
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i * w + j] = a[start + i] === b[start + j] ? lcs[(i + 1) * w + j + 1] + 1 : Math.max(lcs[(i + 1) * w + j], lcs[i * w + j + 1]);
    }
  }
  const hunks: [number, number, number, number][] = [];
  let i = 0;
  let j = 0;
  let hi = -1;
  let hj = -1;
  while (i < n || j < m) {
    if (i < n && j < m && a[start + i] === b[start + j]) {
      if (hi >= 0) hunks.push([start + hi, start + i, start + hj, start + j]);
      hi = -1;
      i++;
      j++;
    } else {
      if (hi < 0) {
        hi = i;
        hj = j;
      }
      if (j < m && (i === n || lcs[i * w + j + 1] >= lcs[(i + 1) * w + j])) j++;
      else i++;
    }
  }
  if (hi >= 0) hunks.push([start + hi, endA, start + hj, endB]);
  return hunks;
}

/** Edits that turn the buffer into `next` while leaving unchanged lines (and the cursor on them) alone. */
function minimalEdits(model: monaco.editor.ITextModel, next: string): monaco.languages.TextEdit[] {
  const a = model.getLinesContent();
  const b = next.split(/\r?\n/);
  const last = a.length;
  const edits: monaco.languages.TextEdit[] = [];
  for (const [a1, a2, b1, b2] of lineHunks(a, b)) {
    const lines = b.slice(b1, b2);
    if (a2 - a1 === b2 - b1) {
      // Same number of lines: edit only the changed middle of each line.
      for (let k = 0; k < lines.length; k++) {
        const x = a[a1 + k];
        const y = lines[k];
        if (x === y) continue;
        let p = 0;
        while (p < x.length && p < y.length && x[p] === y[p]) p++;
        let s = 0;
        while (s < x.length - p && s < y.length - p && x[x.length - 1 - s] === y[y.length - 1 - s]) s++;
        edits.push({ range: new monaco.Range(a1 + k + 1, p + 1, a1 + k + 1, x.length - s + 1), text: y.slice(p, y.length - s) });
      }
    } else if (a2 < last) {
      edits.push({ range: new monaco.Range(a1 + 1, 1, a2 + 1, 1), text: lines.map((l) => `${l}\n`).join('') });
    } else if (a1 > 0) {
      edits.push({ range: new monaco.Range(a1, model.getLineMaxColumn(a1), last, model.getLineMaxColumn(last)), text: lines.map((l) => `\n${l}`).join('') });
    } else {
      edits.push({ range: model.getFullModelRange(), text: lines.join('\n') });
    }
  }
  return edits;
}

monaco.languages.registerDocumentFormattingEditProvider(FORMAT_LANGUAGES, {
  displayName: 'Prettier',
  async provideDocumentFormattingEdits(model, options) {
    const version = model.getVersionId();
    const next = await formattedText(model, options);
    if (next === null || model.isDisposed() || model.getVersionId() !== version) return [];
    return minimalEdits(model, next);
  },
});

/** Formats a buffer in place: through the editor showing it (one undo step, cursor kept), else directly. */
export async function formatBuffer(model: monaco.editor.ITextModel) {
  if (!FORMAT_LANGUAGES.includes(model.getLanguageId())) return;
  const ed = monaco.editor.getEditors().find((e) => e.getModel() === model);
  const action = ed?.getAction('editor.action.formatDocument');
  if (action?.isSupported()) {
    await action.run();
    return;
  }
  const o = model.getOptions();
  const version = model.getVersionId();
  const next = await formattedText(model, { tabSize: o.tabSize, insertSpaces: o.insertSpaces });
  if (next === null || model.isDisposed() || model.getVersionId() !== version) return;
  const edits = minimalEdits(model, next);
  if (edits.length) model.pushEditOperations([], edits, () => null);
}

export async function saveFile(path: string) {
  const m = monaco.editor.getModel(uriFor(path));
  if (!m) return;
  if (useApp.getState().settings?.formatOnSave) await formatBuffer(m);
  try {
    await api.writeFile(path, m.getValue());
    saved.set(path, m.getAlternativeVersionId());
    setDirty(path, false);
    if (path.endsWith('.ts') && useApp.getState().settings?.typeCheckOnSave) scheduleTypecheck();
  } catch (e) {
    toast(errorText(e), 'error');
  }
}

export async function saveAll() {
  for (const t of useApp.getState().tabs) if (t.dirty) await saveFile(t.path);
}

/** Reloads files that changed on disk, unless they have unsaved edits. */
export async function reloadFromDisk(paths: string[]) {
  for (const path of paths) {
    const m = monaco.editor.getModel(uriFor(path));
    if (!m) continue;
    if (isDirty(path)) continue;
    try {
      const content = await api.readFile(path);
      if (content !== m.getValue()) {
        m.pushEditOperations([], [{ range: m.getFullModelRange(), text: content }], () => null);
        saved.set(path, m.getAlternativeVersionId());
        setDirty(path, false);
      }
    } catch {
      // Deleted: leave the buffer as is.
    }
  }
}

const under = (path: string, dir: string) => path === dir || path.startsWith(`${dir}/`);

/** After a rename on disk: reopens the buffers under `from` at their new paths, keeping unsaved edits. */
export async function renameBuffers(from: string, to: string) {
  const moved: { old: string; next: string; unsaved: string | null }[] = [];
  for (const t of useApp.getState().tabs) {
    if (!under(t.path, from)) continue;
    const m = monaco.editor.getModel(uriFor(t.path));
    moved.push({ old: t.path, next: to + t.path.slice(from.length), unsaved: m && isDirty(t.path) ? m.getValue() : null });
  }
  if (!moved.length) return;
  const nextOf = (p: string | null) => moved.find((x) => x.old === p)?.next ?? p;
  useApp.setState((s) => ({ tabs: s.tabs.map((t) => ({ ...t, path: nextOf(t.path)! })), active: nextOf(s.active) }));
  for (const mv of moved) {
    monaco.editor.getModel(uriFor(mv.old))?.dispose();
    saved.delete(mv.old);
    if (mv.unsaved !== null) (await getModel(mv.next)).setValue(mv.unsaved);
  }
}

/** After a delete on disk: closes the tabs under `path` and drops their buffers. */
export function dropBuffers(path: string) {
  const s = useApp.getState();
  const gone = s.tabs.filter((t) => under(t.path, path)).map((t) => t.path);
  if (!gone.length) return;
  useApp.setState((st) => {
    const tabs = st.tabs.filter((t) => !gone.includes(t.path));
    const idx = st.tabs.findIndex((t) => t.path === st.active);
    const active = st.active && gone.includes(st.active) ? (tabs[Math.min(idx, tabs.length - 1)]?.path ?? null) : st.active;
    return { tabs, active };
  });
  for (const p of gone) {
    monaco.editor.getModel(uriFor(p))?.dispose();
    saved.delete(p);
  }
}

export function disposeAllModels() {
  monaco.editor.getModels().forEach((m) => m.dispose());
  saved.clear();
}

// ---- Nest-aware providers --------------------------------------------------------------------
function model(): ProjectModel | null {
  return useApp.getState().model;
}

monaco.editor.registerCommand('nf.sendRequest', (_accessor, method: string, path: string) => {
  const m = model();
  const url = `http://localhost:${m?.port ?? 3000}${path.replace(/:(\w+)/g, '1')}`;
  useApp.setState({ view: 'api', apiDraft: { id: '', name: `${method} ${path}`, method, url, headers: [], body: method === 'GET' || method === 'DELETE' ? '' : '{\n  \n}' } });
});

monaco.editor.registerCommand('nf.openGraph', () => useApp.setState({ view: 'graph' }));

monaco.languages.registerCodeLensProvider('typescript', {
  provideCodeLenses(textModel) {
    const path = pathForUri(textModel.uri);
    const m = model();
    if (!path || !m) return { lenses: [], dispose() {} };
    const lenses: monaco.languages.CodeLens[] = [];
    for (const r of m.routes.filter((x) => x.file === path)) {
      const range = new monaco.Range(r.line, 1, r.line, 1);
      lenses.push({ range, command: { id: 'nf.sendRequest', title: `${r.method} ${r.path}`, arguments: [r.method, r.path] } });
      lenses.push({ range, command: { id: 'nf.sendRequest', title: 'Send request', arguments: [r.method, r.path] } });
    }
    const run = useApp.getState().testRun;
    if (run && path.endsWith('.spec.ts')) {
      const suite = run.suites.find((s) => s.file === path);
      for (const t of suite?.tests ?? []) {
        if (!t.line) continue;
        const title = t.status === 'passed' ? `✓ passed · ${Math.round(t.durationMs)} ms` : t.status === 'failed' ? '✕ failed' : t.status;
        lenses.push({ range: new monaco.Range(t.line, 1, t.line, 1), command: { id: '', title } });
      }
    }
    for (const c of m.classes.filter((x) => x.file === path && x.kind === 'module')) {
      lenses.push({ range: new monaco.Range(c.line, 1, c.line, 1), command: { id: 'nf.openGraph', title: 'Open in module graph' } });
    }
    return { lenses, dispose() {} };
  },
});

monaco.languages.registerDefinitionProvider('typescript', {
  provideDefinition(textModel, position) {
    const word = textModel.getWordAtPosition(position)?.word;
    const m = model();
    if (!word || !m) return null;
    const cls = m.classes.find((c) => c.name === word);
    if (!cls) return null;
    const here = pathForUri(textModel.uri);
    if (cls.file === here) return null;
    return { uri: uriFor(cls.file), range: new monaco.Range(cls.line, 1, cls.line, 1) };
  },
});

monaco.languages.registerHoverProvider('typescript', {
  provideHover(textModel, position) {
    const word = textModel.getWordAtPosition(position)?.word;
    const m = model();
    if (!word || !m) return null;
    const cls = m.classes.find((c) => c.name === word);
    if (!cls) return null;
    const mod = cls.modules.length ? `provided in **${cls.modules.join(', ')}**` : cls.kind === 'module' ? '' : '_not registered in any module_';
    const injects = cls.injects.length ? `\n\ninjects ${cls.injects.map((i) => `\`${i.token}\``).join(', ')}` : '';
    return { contents: [{ value: `**${cls.name}** · ${cls.kind} ${mod}${injects}\n\n\`${cls.file}:${cls.line}\`` }] };
  },
});

monaco.editor.registerEditorOpener({
  openCodeEditor(_source, resource, selectionOrPosition) {
    const path = pathForUri(resource);
    if (!path) return false;
    const line = selectionOrPosition ? ('startLineNumber' in selectionOrPosition ? selectionOrPosition.startLineNumber : selectionOrPosition.lineNumber) : undefined;
    openFile(path, line);
    return true;
  },
});

/** Graph diagnostics and tsc problems as editor markers. */
export function updateMarkers() {
  const s = useApp.getState();
  const m = s.model;
  for (const tm of monaco.editor.getModels()) {
    const path = pathForUri(tm.uri);
    if (!path) continue;
    const markers: monaco.editor.IMarkerData[] = [];
    for (const d of m?.diagnostics ?? []) {
      if (d.file !== path || !d.line) continue;
      const lineLen = d.line <= tm.getLineCount() ? tm.getLineMaxColumn(d.line) : 1;
      markers.push({
        severity: d.severity === 'error' ? monaco.MarkerSeverity.Error : d.severity === 'warning' ? monaco.MarkerSeverity.Warning : monaco.MarkerSeverity.Info,
        message: `${d.title}: ${d.subject}\n${d.detail}`,
        source: 'NestFactory',
        startLineNumber: d.line,
        endLineNumber: d.line,
        startColumn: 1,
        endColumn: lineLen,
      });
    }
    for (const p of s.problems) {
      if (p.file !== path) continue;
      markers.push({
        severity: p.severity === 'error' ? monaco.MarkerSeverity.Error : monaco.MarkerSeverity.Warning,
        message: p.message,
        code: p.code,
        source: 'tsc',
        startLineNumber: p.line,
        endLineNumber: p.line,
        startColumn: p.column,
        endColumn: p.column + 1,
      });
    }
    monaco.editor.setModelMarkers(tm, 'nestfactory', markers);
  }
}

useApp.subscribe((s, prev) => {
  if (s.model !== prev.model || s.problems !== prev.problems) updateMarkers();
});
