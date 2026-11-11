// IntelliSense from the project's own TypeScript server (tsserver), as in VS Code. Monaco's
// built-in TypeScript only sees open buffers and no node_modules, so it can't complete or import
// `UsersService` from another folder or `Injectable` from @nestjs/common. tsserver sees the whole
// project through its tsconfig: completions with auto-imports, quick fixes ("Add import", "Add all
// missing imports"), organize imports, rename across files, references, live diagnostics, and
// import updates when files move. Without it (no node_modules yet) Monaco's own support stays on.
//
// Buffers are mirrored into tsserver with `updateOpen`: opens, edits and closes are queued and
// sent ahead of the next request. Requests go one at a time, so edits always arrive in order;
// background work (diagnostics) waits behind what the user is waiting for (completions, hovers).
import * as monaco from 'monaco-editor';
import * as tsMode from 'monaco-editor/languages/features/typescript/tsMode';
import { api, on } from './api';
import { useApp, toast, errorText } from './store';
import { uriFor, pathForUri, getModel, saveParticipants, updateMarkers } from './monaco';

// ---- protocol (the subset used here) ----------------------------------------------------------
interface Loc {
  line: number;
  offset: number;
}
interface Span {
  start: Loc;
  end: Loc;
}
interface FileSpan extends Span {
  file: string;
  contextStart?: Loc;
  contextEnd?: Loc;
}
interface CodeEdit extends Span {
  newText: string;
}
export interface FileCodeEdits {
  fileName: string;
  textChanges: CodeEdit[];
  isNewFile?: boolean;
}
interface CodeAction {
  description: string;
  changes: FileCodeEdits[];
}
interface CodeFixAction extends CodeAction {
  fixName: string;
  fixId?: string;
  fixAllDescription?: string;
}
interface DisplayPart {
  text: string;
  kind: string;
}
interface JsDocTag {
  name: string;
  text?: string | DisplayPart[];
}
interface CompletionEntry {
  name: string;
  kind: string;
  kindModifiers?: string;
  sortText: string;
  insertText?: string;
  filterText?: string;
  isSnippet?: true;
  replacementSpan?: Span;
  hasAction?: true;
  source?: string;
  sourceDisplay?: DisplayPart[];
  labelDetails?: { detail?: string; description?: string };
  isRecommended?: true;
  data?: unknown;
}
interface CompletionInfo {
  entries: CompletionEntry[];
  isIncomplete?: true;
  optionalReplacementSpan?: Span;
}
interface CompletionDetails {
  name: string;
  displayParts: DisplayPart[];
  documentation?: DisplayPart[];
  tags?: JsDocTag[];
  codeActions?: CodeAction[];
}
interface Diagnostic extends Span {
  text: string;
  code: number;
  category: 'error' | 'warning' | 'suggestion' | 'message';
  reportsUnnecessary?: true;
  reportsDeprecated?: true;
  relatedInformation?: { span?: FileSpan; message: string }[];
}
interface NavTree {
  text: string;
  kind: string;
  kindModifiers: string;
  spans: { start: Loc; end: Loc }[];
  nameSpan?: Span;
  childItems?: NavTree[];
}

const NOT_RUNNING = "The TypeScript server isn't running";
const LANGS = ['typescript', 'javascript'];
/** Marker owner for live diagnostics (Monaco's own TypeScript uses 'typescript' and clears it). */
const OWNER = 'tsserver';

// ---- state ----------------------------------------------------------------------------------------
let active = false;
let rootFwd = '';
let generation = 0;
const tracked = new Map<string, monaco.editor.ITextModel>();
const toOpen = new Set<string>();
const toClose = new Set<string>();
/** Edits in the order they happened, one entry per change event. */
let changes: { rel: string; textChanges: CodeEdit[] }[] = [];
const dropChanges = (rel: string) => (changes = changes.filter((c) => c.rel !== rel));
const listening = new WeakSet<monaco.editor.ITextModel>();

const fwd = (p: string) => p.replace(/\\/g, '/');
const tsFile = (rel: string) => `${rootFwd}/${rel}`;

/** A tsserver file name as a project-relative path, or null outside the project. */
function relOf(file: string): string | null {
  const f = fwd(file);
  const r = rootFwd.toLowerCase();
  return f.toLowerCase().startsWith(`${r}/`) ? f.slice(r.length + 1) : null;
}

const isCode = (m: monaco.editor.ITextModel) => LANGS.includes(m.getLanguageId());

/** The project path of a buffer tsserver is serving, or null (other languages, diff views, no server). */
function served(m: monaco.editor.ITextModel): string | null {
  if (!active) return null;
  const rel = pathForUri(m.uri);
  return rel && tracked.get(rel) === m ? rel : null;
}

const toRange = (s: Loc, e: Loc) => new monaco.Range(s.line, s.offset, e.line, e.offset);
const at = (rel: string, p: monaco.IPosition) => ({ file: tsFile(rel), line: p.lineNumber, offset: p.column });
const text = (parts?: DisplayPart[] | string) => (typeof parts === 'string' ? parts : (parts ?? []).map((p) => p.text).join(''));

// ---- request queue ------------------------------------------------------------------------------------
class Cancelled extends Error {}
type Job = { low: boolean; run: () => Promise<void> };
const queue: Job[] = [];
let busy = false;

function pump() {
  if (busy) return;
  const job = queue.shift();
  if (!job) return;
  busy = true;
  void job.run().finally(() => {
    busy = false;
    pump();
  });
}

/** The buffered opens, edits and closes as `updateOpen` arguments, or null when there are none. */
function takeSync() {
  if (!toOpen.size && !changes.length && !toClose.size) return null;
  const openFiles = [...toOpen]
    .map((rel) => tracked.get(rel))
    .filter((m): m is monaco.editor.ITextModel => !!m && !m.isDisposed())
    .map((m) => ({ file: tsFile(pathForUri(m.uri)!), fileContent: m.getValue(), projectRootPath: rootFwd }));
  const changedFiles = changes.map((c) => ({ fileName: tsFile(c.rel), textChanges: c.textChanges }));
  const closedFiles = [...toClose].map(tsFile);
  toOpen.clear();
  changes = [];
  toClose.clear();
  return { openFiles, changedFiles, closedFiles };
}

interface Opts {
  token?: monaco.CancellationToken;
  low?: boolean;
  timeoutMs?: number;
}

export function request<T>(command: string, args: object, opts: Opts = {}): Promise<T> {
  if (!active) return Promise.reject(new Error(NOT_RUNNING));
  const gen = generation;
  return new Promise<T>((resolve, reject) => {
    const job: Job = {
      low: !!opts.low,
      run: async () => {
        if (opts.token?.isCancellationRequested || gen !== generation) return reject(new Cancelled());
        try {
          resolve(await api.tsRequest<T>(command, args, takeSync(), opts.timeoutMs));
        } catch (e) {
          if (errorText(e).includes(NOT_RUNNING) && gen === generation) void restart();
          reject(e);
        }
      },
    };
    // Interactive requests go ahead of queued background work; the buffered edits travel with
    // whichever request goes first, so the order of edits is kept either way.
    const i = job.low ? -1 : queue.findIndex((j) => j.low);
    if (i < 0) queue.push(job);
    else queue.splice(i, 0, job);
    pump();
  });
}

/** Runs a provider body, answering nothing (rather than an error popup) when tsserver can't. */
async function quietly<T>(fn: () => Promise<T>): Promise<T | undefined> {
  try {
    return await fn();
  } catch {
    return undefined;
  }
}

// ---- buffers ----------------------------------------------------------------------------------------
function track(m: monaco.editor.ITextModel) {
  if (!active || m.isDisposed() || !isCode(m)) return;
  const rel = pathForUri(m.uri);
  if (!rel || tracked.get(rel) === m) return;
  tracked.set(rel, m);
  toClose.delete(rel);
  dropChanges(rel);
  toOpen.add(rel);
  if (listening.has(m)) return;
  listening.add(m);
  m.onDidChangeContent((e) => {
    const r = pathForUri(m.uri);
    if (!active || !r || tracked.get(r) !== m) return;
    if (e.isFlush) {
      dropChanges(r);
      toOpen.add(r); // Reopened with the new content.
    } else if (!toOpen.has(r)) {
      // Monaco lists one event's edits last-in-document first, so they apply in that order;
      // tsserver applies each list back to front, so it's sent reversed (as VS Code does).
      const textChanges = e.changes.map((c) => ({ start: { line: c.range.startLineNumber, offset: c.range.startColumn }, end: { line: c.range.endLineNumber, offset: c.range.endColumn }, newText: c.text }));
      changes.push({ rel: r, textChanges: textChanges.reverse() });
    }
    scheduleDiagnostics();
  });
  m.onWillDispose(() => {
    const r = pathForUri(m.uri);
    if (!r || tracked.get(r) !== m) return;
    tracked.delete(r);
    dropChanges(r);
    if (toOpen.has(r)) toOpen.delete(r);
    else if (active) toClose.add(r);
  });
}

monaco.editor.onDidCreateModel(track);
monaco.editor.onDidCreateEditor((ed) => {
  ed.onDidChangeModel(() => scheduleDiagnostics(30));
});

// ---- lifecycle ------------------------------------------------------------------------------------------
// Monaco reads its TypeScript mode configuration once, when the first TypeScript buffer opens, so
// its own providers are switched off here, before any buffer exists, and registered by hand
// (`useWorker`) only while the project's server isn't available. Formatting stays Monaco's.
const WORKER_FEATURES = ['completionItems', 'hovers', 'documentSymbols', 'definitions', 'references', 'documentHighlights', 'rename', 'diagnostics', 'signatureHelp', 'codeActions', 'inlayHints'] as const;
for (const d of [monaco.typescript.typescriptDefaults, monaco.typescript.javascriptDefaults]) {
  d.setModeConfiguration({ ...d.modeConfiguration, ...Object.fromEntries(WORKER_FEATURES.map((k) => [k, false])) });
}

let fallback: monaco.IDisposable[] = [];

/** Monaco's in-browser TypeScript (open buffers only, no node_modules): the fallback IntelliSense. */
function useWorker(on: boolean) {
  if (on === fallback.length > 0) return;
  fallback.forEach((d) => d.dispose());
  fallback = [];
  if (!on) return;
  const L = monaco.languages;
  for (const [lang, get, defaults] of [
    ['typescript', monaco.typescript.getTypeScriptWorker, monaco.typescript.typescriptDefaults],
    ['javascript', monaco.typescript.getJavaScriptWorker, monaco.typescript.javascriptDefaults],
  ] as const) {
    const worker = async (...uris: monaco.Uri[]) => (await get())(...uris);
    const libs = new tsMode.LibFiles(worker);
    fallback.push(
      L.registerCompletionItemProvider(lang, new tsMode.SuggestAdapter(worker)),
      L.registerSignatureHelpProvider(lang, new tsMode.SignatureHelpAdapter(worker)),
      L.registerHoverProvider(lang, new tsMode.QuickInfoAdapter(worker)),
      L.registerDocumentHighlightProvider(lang, new tsMode.DocumentHighlightAdapter(worker)),
      L.registerDefinitionProvider(lang, new tsMode.DefinitionAdapter(libs, worker)),
      L.registerReferenceProvider(lang, new tsMode.ReferenceAdapter(libs, worker)),
      L.registerDocumentSymbolProvider(lang, new tsMode.OutlineAdapter(worker)),
      L.registerRenameProvider(lang, new tsMode.RenameAdapter(libs, worker)),
      L.registerCodeActionProvider(lang, new tsMode.CodeActionAdaptor(worker)),
      new tsMode.DiagnosticsAdapter(libs, defaults, lang, worker),
    );
  }
}
useWorker(true);

function reset() {
  generation++;
  queue.splice(0).forEach((j) => void j.run()); // Each rejects: its generation is gone.
  tracked.clear();
  toOpen.clear();
  toClose.clear();
  changes = [];
  for (const m of monaco.editor.getModels()) monaco.editor.setModelMarkers(m, OWNER, []);
}

async function start() {
  const root = useApp.getState().model?.root;
  active = false;
  reset();
  if (!root) {
    useWorker(true);
    useApp.setState({ ts: { status: 'off', version: null, error: null } });
    return;
  }
  const gen = generation;
  rootFwd = fwd(root).replace(/\/+$/, '');
  useApp.setState({ ts: { status: 'starting', version: null, error: null } });
  try {
    const info = await api.tsStart();
    if (gen !== generation) return;
    active = true;
    useWorker(false);
    for (const m of monaco.editor.getModels()) track(m);
    useApp.setState({ ts: { status: 'ready', version: info.version, error: null } });
    scheduleDiagnostics(0);
  } catch (e) {
    if (gen !== generation) return;
    useWorker(true);
    useApp.setState({ ts: { status: 'error', version: null, error: errorText(e) } });
  }
}

const restarts: number[] = [];
let restarting = false;
/** After tsserver exits (out of memory, killed): start a new one and reopen every buffer. */
export async function restart(manual = false) {
  if (restarting) return;
  const now = Date.now();
  while (restarts.length && now - restarts[0] > 120_000) restarts.shift();
  if (!manual && restarts.length >= 3) {
    active = false;
    useWorker(true);
    useApp.setState({ ts: { status: 'error', version: null, error: 'The TypeScript server keeps stopping. Click to restart it.' } });
    return;
  }
  restarts.push(now);
  restarting = true;
  try {
    await start();
  } finally {
    restarting = false;
  }
}

let lastRoot: string | undefined;
function onProject() {
  const root = useApp.getState().model?.root;
  if (root === lastRoot) return;
  lastRoot = root;
  restarts.length = 0;
  void start();
}
useApp.subscribe(onProject);
onProject();

// ---- diagnostics ---------------------------------------------------------------------------------------
let diagTimer: ReturnType<typeof setTimeout> | undefined;
function scheduleDiagnostics(delay = 350) {
  clearTimeout(diagTimer);
  diagTimer = setTimeout(() => void runDiagnostics(), delay);
}

const SEVERITY = { error: monaco.MarkerSeverity.Error, warning: monaco.MarkerSeverity.Warning, suggestion: monaco.MarkerSeverity.Hint, message: monaco.MarkerSeverity.Info };

function toMarker(d: Diagnostic): monaco.editor.IMarkerData {
  const tags: monaco.MarkerTag[] = [];
  if (d.reportsUnnecessary) tags.push(monaco.MarkerTag.Unnecessary);
  if (d.reportsDeprecated) tags.push(monaco.MarkerTag.Deprecated);
  const related = (d.relatedInformation ?? []).flatMap((r) => {
    const rel = r.span && relOf(r.span.file);
    return rel && r.span ? [{ resource: uriFor(rel), message: r.message, ...rangeFields(r.span) }] : [];
  });
  return { severity: SEVERITY[d.category] ?? monaco.MarkerSeverity.Info, message: d.text, code: String(d.code), source: 'ts', tags, relatedInformation: related, ...rangeFields(d) };
}

function rangeFields(s: Span) {
  return { startLineNumber: s.start.line, startColumn: s.start.offset, endLineNumber: s.end.line, endColumn: s.end.offset };
}

/** Live errors, warnings and unused-code hints for the files on screen. */
async function runDiagnostics() {
  if (!active) return;
  const shown = new Set(monaco.editor.getEditors().map((e) => e.getModel()));
  for (const m of shown) {
    if (!m) continue;
    const rel = served(m);
    if (!rel || /(^|\/)node_modules\//.test(rel) || rel.endsWith('.d.ts')) continue;
    const version = m.getVersionId();
    const args = { file: tsFile(rel) };
    const all = await quietly(() =>
      Promise.all([
        request<Diagnostic[]>('syntacticDiagnosticsSync', args, { low: true }),
        request<Diagnostic[]>('semanticDiagnosticsSync', args, { low: true }),
        request<Diagnostic[]>('suggestionDiagnosticsSync', args, { low: true }),
      ]),
    );
    // Edited meanwhile: the edit scheduled a fresh run.
    if (!all || m.isDisposed() || m.getVersionId() !== version) continue;
    monaco.editor.setModelMarkers(m, OWNER, all.flat().map(toMarker));
  }
}

on('files-changed', () => scheduleDiagnostics(1200));
// Installing the project's packages brings TypeScript; try again rather than wait for a click.
on('task-done', (p: { id: string; ok: boolean }) => {
  if (p.ok && (p.id === 'install' || p.id === 'packages') && useApp.getState().ts.status === 'error') void restart(true);
});

// ---- applying edits ---------------------------------------------------------------------------------
/** Models for the files an edit touches (Monaco applies workspace edits only to loaded models). */
async function workspaceEdit(changes: FileCodeEdits[]): Promise<monaco.languages.WorkspaceEdit> {
  const edits: monaco.languages.IWorkspaceTextEdit[] = [];
  for (const c of changes) {
    const rel = relOf(c.fileName);
    if (!rel || c.isNewFile) continue;
    if (!(await quietly(() => getModel(rel)))) continue;
    for (const t of c.textChanges) edits.push({ resource: uriFor(rel), textEdit: { range: toRange(t.start, t.end), text: t.newText }, versionId: undefined });
  }
  return { edits };
}

/** Applies tsserver edits: in the editor showing the file (one undo step), else to its buffer. */
export async function applyFileEdits(changes: FileCodeEdits[], mapPath: (rel: string) => string = (r) => r) {
  let files = 0;
  for (const c of changes) {
    let rel = relOf(c.fileName);
    if (!rel || !c.textChanges.length) continue;
    rel = mapPath(rel);
    if (c.isNewFile) await api.writeFile(rel, '');
    const m = await quietly(() => getModel(rel!));
    if (!m) continue;
    const edits = c.textChanges.map((t) => ({ range: toRange(t.start, t.end), text: t.newText, forceMoveMarkers: true }));
    const ed = monaco.editor.getEditors().find((e) => e.getModel() === m);
    if (ed) {
      ed.pushUndoStop();
      ed.executeEdits('typescript', edits);
      ed.pushUndoStop();
    } else {
      m.pushEditOperations([], edits, () => null);
    }
    files++;
  }
  return files;
}

// ---- completions -----------------------------------------------------------------------------------------
const K = monaco.languages.CompletionItemKind;
const COMPLETION_KIND: Record<string, monaco.languages.CompletionItemKind> = {
  keyword: K.Keyword,
  'primitive type': K.Keyword,
  const: K.Constant,
  let: K.Variable,
  var: K.Variable,
  'local var': K.Variable,
  alias: K.Variable,
  parameter: K.Variable,
  property: K.Field,
  getter: K.Field,
  setter: K.Field,
  accessor: K.Field,
  function: K.Function,
  'local function': K.Function,
  method: K.Method,
  construct: K.Method,
  call: K.Method,
  index: K.Method,
  constructor: K.Constructor,
  enum: K.Enum,
  'enum member': K.EnumMember,
  module: K.Module,
  'external module name': K.Module,
  class: K.Class,
  'local class': K.Class,
  type: K.Class,
  interface: K.Interface,
  'type parameter': K.TypeParameter,
  script: K.File,
  directory: K.Folder,
  string: K.Constant,
  warning: K.Text,
};

const TRIGGERS = ['.', '"', "'", '`', '/', '@', '<', '#', ' '];
const META = Symbol('ts');
type Item = monaco.languages.CompletionItem & { [META]?: { rel: string; line: number; offset: number; entry: CompletionEntry } };

function tagsMarkdown(tags?: JsDocTag[]) {
  return (tags ?? [])
    .map((t) => {
      const body = text(t.text);
      if (t.name === 'example') return `*@example*\n\`\`\`typescript\n${body}\n\`\`\``;
      if ((t.name === 'param' || t.name === 'template') && body) {
        const [name, ...rest] = body.split(/\s+/);
        return `*@${t.name}* \`${name}\`${rest.length ? ` — ${rest.join(' ')}` : ''}`;
      }
      return `*@${t.name}*${body ? ` — ${body}` : ''}`;
    })
    .join('\n\n');
}

function docs(documentation?: DisplayPart[] | string, tags?: JsDocTag[]) {
  return [text(documentation), tagsMarkdown(tags)].filter(Boolean).join('\n\n');
}

monaco.languages.registerCompletionItemProvider(LANGS, {
  triggerCharacters: TRIGGERS,
  async provideCompletionItems(model, position, context, token) {
    const rel = served(model);
    if (!rel) return undefined;
    const trigger = context.triggerCharacter && TRIGGERS.includes(context.triggerCharacter) ? context.triggerCharacter : undefined;
    const body = await quietly(() =>
      request<CompletionInfo | undefined>(
        'completionInfo',
        { ...at(rel, position), includeExternalModuleExports: true, includeInsertTextCompletions: true, triggerCharacter: trigger, triggerKind: context.triggerKind + 1 },
        { token },
      ),
    );
    if (!body) return { suggestions: [] };
    const word = model.getWordUntilPosition(position);
    const wordEnd = model.getWordAtPosition(position)?.endColumn ?? position.column;
    const line = position.lineNumber;
    let range: monaco.languages.CompletionItemRanges | monaco.IRange = {
      insert: new monaco.Range(line, word.startColumn, line, position.column),
      replace: new monaco.Range(line, word.startColumn, line, wordEnd),
    };
    const opt = body.optionalReplacementSpan;
    if (opt && opt.start.line === line && opt.end.line === line && opt.start.offset <= position.column) {
      range = { insert: new monaco.Range(line, opt.start.offset, line, position.column), replace: toRange(opt.start, opt.end) };
    }
    const suggestions = body.entries.map((entry): Item => {
      const mods = new Set((entry.kindModifiers ?? '').split(/[,\s]+/));
      const autoImport = entry.hasAction && entry.source ? text(entry.sourceDisplay) || entry.source : undefined;
      const item: Item = {
        label: { label: entry.name + (mods.has('optional') ? '?' : ''), detail: entry.labelDetails?.detail, description: entry.labelDetails?.description ?? autoImport },
        kind: COMPLETION_KIND[entry.kind] ?? K.Property,
        sortText: entry.sortText,
        insertText: entry.insertText ?? entry.name,
        insertTextRules: entry.isSnippet ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet : undefined,
        filterText: entry.filterText ?? (entry.replacementSpan && entry.insertText ? entry.insertText : undefined),
        range: entry.replacementSpan ? toRange(entry.replacementSpan.start, entry.replacementSpan.end) : range,
        preselect: entry.isRecommended,
        tags: mods.has('deprecated') ? [monaco.languages.CompletionItemTag.Deprecated] : undefined,
      };
      item[META] = { rel, line, offset: position.column, entry };
      return item;
    });
    return { suggestions, incomplete: !!body.isIncomplete };
  },
  async resolveCompletionItem(item: Item, token) {
    const meta = item[META];
    if (!meta || !active) return item;
    const { rel, line, offset, entry } = meta;
    const details = await quietly(() =>
      request<CompletionDetails[]>('completionEntryDetails', { file: tsFile(rel), line, offset, entryNames: [{ name: entry.name, source: entry.source, data: entry.data }] }, { token }),
    );
    const d = details?.[0];
    if (!d) return item;
    const actions = d.codeActions ?? [];
    item.detail = [...actions.map((a) => a.description), text(d.displayParts)].filter(Boolean).join('\n');
    const doc = docs(d.documentation, d.tags);
    if (doc) item.documentation = { value: doc };
    // The import the completion needs ("Add import from '@nestjs/common'").
    const edits = actions.flatMap((a) => a.changes.filter((c) => relOf(c.fileName) === rel).flatMap((c) => c.textChanges.map((t) => ({ range: toRange(t.start, t.end), text: t.newText }))));
    if (edits.length) item.additionalTextEdits = edits;
    return item;
  },
});

// ---- hover, signatures ------------------------------------------------------------------------------------
monaco.languages.registerHoverProvider(LANGS, {
  async provideHover(model, position, token) {
    const rel = served(model);
    if (!rel) return undefined;
    const q = await quietly(() => request<{ displayString: string; documentation: string | DisplayPart[]; tags?: JsDocTag[]; start: Loc; end: Loc }>('quickinfo', at(rel, position), { token }));
    if (!q || !q.displayString) return undefined;
    const doc = docs(q.documentation, q.tags);
    return { range: toRange(q.start, q.end), contents: [{ value: '```typescript\n' + q.displayString + '\n```' }, ...(doc ? [{ value: doc }] : [])] };
  },
});

interface SigItem {
  prefixDisplayParts: DisplayPart[];
  suffixDisplayParts: DisplayPart[];
  separatorDisplayParts: DisplayPart[];
  parameters: { displayParts: DisplayPart[]; documentation: DisplayPart[] }[];
  documentation: DisplayPart[];
  tags: JsDocTag[];
}

monaco.languages.registerSignatureHelpProvider(LANGS, {
  signatureHelpTriggerCharacters: ['(', ',', '<'],
  signatureHelpRetriggerCharacters: [')'],
  async provideSignatureHelp(model, position, token, context) {
    const rel = served(model);
    if (!rel) return undefined;
    const ch = context.triggerCharacter;
    const triggerReason =
      context.triggerKind === monaco.languages.SignatureHelpTriggerKind.Invoke
        ? { kind: 'invoked' }
        : context.triggerKind === monaco.languages.SignatureHelpTriggerKind.TriggerCharacter && ch
          ? { kind: context.isRetrigger ? 'retrigger' : 'characterTyped', triggerCharacter: ch }
          : { kind: 'retrigger' };
    const body = await quietly(() => request<{ items: SigItem[]; selectedItemIndex: number; argumentIndex: number }>('signatureHelp', { ...at(rel, position), triggerReason }, { token }));
    if (!body?.items.length) return undefined;
    const signatures = body.items.map((item) => {
      let label = text(item.prefixDisplayParts);
      const parameters = item.parameters.map((p, i) => {
        const start = label.length;
        label += text(p.displayParts);
        const end = label.length;
        if (i < item.parameters.length - 1) label += text(item.separatorDisplayParts);
        return { label: [start, end] as [number, number], documentation: { value: text(p.documentation) } };
      });
      label += text(item.suffixDisplayParts);
      return { label, parameters, documentation: { value: docs(item.documentation, item.tags) } };
    });
    return { value: { signatures, activeSignature: body.selectedItemIndex, activeParameter: body.argumentIndex }, dispose() {} };
  },
});

// ---- navigation --------------------------------------------------------------------------------------------
/** Locations in project files (node_modules included), with their buffers loaded for peek views. */
async function locations(spans: FileSpan[]): Promise<monaco.languages.LocationLink[]> {
  const out = await Promise.all(
    spans.map(async (s): Promise<monaco.languages.LocationLink | null> => {
      const rel = relOf(s.file);
      if (!rel || !(await quietly(() => getModel(rel)))) return null;
      const target = toRange(s.start, s.end);
      return { uri: uriFor(rel), range: s.contextStart && s.contextEnd ? toRange(s.contextStart, s.contextEnd) : target, targetSelectionRange: target };
    }),
  );
  return out.filter((x): x is monaco.languages.LocationLink => !!x);
}

monaco.languages.registerDefinitionProvider(LANGS, {
  async provideDefinition(model, position, token) {
    const rel = served(model);
    if (!rel) return undefined;
    const body = await quietly(() => request<{ definitions: FileSpan[]; textSpan: Span }>('definitionAndBoundSpan', at(rel, position), { token }));
    if (!body?.definitions?.length) return undefined;
    const origin = toRange(body.textSpan.start, body.textSpan.end);
    return (await locations(body.definitions)).map((l) => ({ ...l, originSelectionRange: origin }));
  },
});

monaco.languages.registerTypeDefinitionProvider(LANGS, {
  async provideTypeDefinition(model, position, token) {
    const rel = served(model);
    if (!rel) return undefined;
    const spans = await quietly(() => request<FileSpan[]>('typeDefinition', at(rel, position), { token }));
    return spans ? locations(spans) : undefined;
  },
});

monaco.languages.registerImplementationProvider(LANGS, {
  async provideImplementation(model, position, token) {
    const rel = served(model);
    if (!rel) return undefined;
    const spans = await quietly(() => request<FileSpan[]>('implementation', at(rel, position), { token }));
    return spans ? locations(spans) : undefined;
  },
});

monaco.languages.registerReferenceProvider(LANGS, {
  async provideReferences(model, position, context, token) {
    const rel = served(model);
    if (!rel) return undefined;
    const body = await quietly(() => request<{ refs: (FileSpan & { isDefinition?: boolean })[] }>('references', at(rel, position), { token }));
    if (!body) return undefined;
    const refs = body.refs.filter((r) => context.includeDeclaration || !r.isDefinition);
    return (await locations(refs)).map((l) => ({ uri: l.uri, range: l.targetSelectionRange ?? l.range }));
  },
});

monaco.languages.registerDocumentHighlightProvider(LANGS, {
  async provideDocumentHighlights(model, position, token) {
    const rel = served(model);
    if (!rel) return undefined;
    const file = tsFile(rel);
    const body = await quietly(() => request<{ file: string; highlightSpans: (Span & { kind: string })[] }[]>('documentHighlights', { ...at(rel, position), filesToSearch: [file] }, { token }));
    const own = body?.find((b) => relOf(b.file) === rel);
    return own?.highlightSpans.map((s) => ({ range: toRange(s.start, s.end), kind: s.kind === 'writtenReference' ? monaco.languages.DocumentHighlightKind.Write : monaco.languages.DocumentHighlightKind.Read }));
  },
});

const S = monaco.languages.SymbolKind;
const SYMBOL_KIND: Record<string, monaco.languages.SymbolKind> = {
  module: S.Module,
  class: S.Class,
  'local class': S.Class,
  enum: S.Enum,
  'enum member': S.EnumMember,
  interface: S.Interface,
  type: S.TypeParameter,
  method: S.Method,
  property: S.Property,
  getter: S.Property,
  setter: S.Property,
  constructor: S.Constructor,
  function: S.Function,
  'local function': S.Function,
  const: S.Constant,
};

function symbol(t: NavTree): monaco.languages.DocumentSymbol | null {
  if (t.kind === 'alias' || !t.spans.length) return null;
  const first = t.spans[0];
  const last = t.spans[t.spans.length - 1];
  const range = toRange(first.start, last.end);
  return {
    name: t.text,
    detail: '',
    kind: SYMBOL_KIND[t.kind] ?? S.Variable,
    tags: t.kindModifiers.includes('deprecated') ? [monaco.languages.SymbolTag.Deprecated] : [],
    range,
    selectionRange: t.nameSpan ? toRange(t.nameSpan.start, t.nameSpan.end) : range,
    children: (t.childItems ?? []).map(symbol).filter((x): x is monaco.languages.DocumentSymbol => !!x),
  };
}

monaco.languages.registerDocumentSymbolProvider(LANGS, {
  displayName: 'TypeScript',
  async provideDocumentSymbols(model, token) {
    const rel = served(model);
    if (!rel) return undefined;
    const tree = await quietly(() => request<NavTree>('navtree', { file: tsFile(rel) }, { token }));
    return (tree?.childItems ?? []).map(symbol).filter((x): x is monaco.languages.DocumentSymbol => !!x);
  },
});

// ---- rename --------------------------------------------------------------------------------------------------
interface RenameBody {
  info: { canRename: boolean; localizedErrorMessage?: string; triggerSpan?: Span };
  locs: { file: string; locs: (Span & { prefixText?: string; suffixText?: string })[] }[];
}

monaco.languages.registerRenameProvider(LANGS, {
  async resolveRenameLocation(model, position, token) {
    const rel = served(model);
    if (!rel) return undefined;
    const body = await quietly(() => request<RenameBody>('rename', { ...at(rel, position), findInStrings: false, findInComments: false }, { token }));
    const span = body?.info.triggerSpan;
    if (!body?.info.canRename || !span) {
      const w = model.getWordAtPosition(position);
      return { range: new monaco.Range(position.lineNumber, w?.startColumn ?? position.column, position.lineNumber, w?.endColumn ?? position.column), text: w?.word ?? '', rejectReason: body?.info.localizedErrorMessage ?? 'You cannot rename this element.' };
    }
    const range = toRange(span.start, span.end);
    return { range, text: model.getValueInRange(range) };
  },
  async provideRenameEdits(model, position, newName, token) {
    const rel = served(model);
    if (!rel) return undefined;
    const body = await quietly(() => request<RenameBody>('rename', { ...at(rel, position), findInStrings: false, findInComments: false }, { token }));
    if (!body?.info.canRename) return { edits: [], rejectReason: body?.info.localizedErrorMessage ?? 'You cannot rename this element.' };
    const edits: monaco.languages.IWorkspaceTextEdit[] = [];
    for (const group of body.locs) {
      const r = relOf(group.file);
      if (!r || !(await quietly(() => getModel(r)))) continue;
      for (const l of group.locs) edits.push({ resource: uriFor(r), textEdit: { range: toRange(l.start, l.end), text: `${l.prefixText ?? ''}${newName}${l.suffixText ?? ''}` }, versionId: undefined });
    }
    return { edits };
  },
});

// ---- code actions: quick fixes, refactors, source actions --------------------------------------------------
const FIX_ALL = 'nf.ts.fixAll';
const REFACTOR = 'nf.ts.refactor';
const ADD_MISSING = 'nf.ts.addMissingImports';

monaco.editor.registerCommand(FIX_ALL, async (_a, rel: string, fixId: string) => {
  const body = await quietly(() => request<{ changes: FileCodeEdits[] }>('getCombinedCodeFix', { scope: { type: 'file', args: { file: tsFile(rel) } }, fixId }));
  if (body) await applyFileEdits(body.changes);
});

monaco.editor.registerCommand(ADD_MISSING, async (_a, rel: string) => {
  const m = monaco.editor.getModel(uriFor(rel));
  if (m) await addMissingImports(m);
});

monaco.editor.registerCommand(REFACTOR, async (_a, rel: string, span: { startLine: number; startOffset: number; endLine: number; endOffset: number }, refactor: string, action: string) => {
  try {
    const body = await request<{ edits: FileCodeEdits[]; renameFilename?: string; renameLocation?: Loc }>('getEditsForRefactor', { file: tsFile(rel), ...span, refactor, action });
    await applyFileEdits(body.edits);
    // "Extract to constant" and the like: rename the new name right away, as VS Code does.
    const target = body.renameFilename ? relOf(body.renameFilename) : null;
    const ed = monaco.editor.getEditors().find((e) => e.getModel() && pathForUri(e.getModel()!.uri) === target);
    if (ed && body.renameLocation) {
      ed.setPosition({ lineNumber: body.renameLocation.line, column: body.renameLocation.offset });
      ed.focus();
      void ed.getAction('editor.action.rename')?.run();
    }
  } catch (e) {
    toast(errorText(e), 'error');
  }
});

/** Adds imports for every name the file uses but doesn't import (TypeScript's "Add all missing imports"). */
export async function addMissingImports(m: monaco.editor.ITextModel, token?: monaco.CancellationToken) {
  const rel = served(m);
  if (!rel) return 0;
  const version = m.getVersionId();
  const body = await request<{ changes: FileCodeEdits[] }>('getCombinedCodeFix', { scope: { type: 'file', args: { file: tsFile(rel) } }, fixId: 'fixMissingImport' }, { timeoutMs: 10_000, token });
  // Edited (or the save went ahead) meanwhile: the edits no longer fit.
  if (token?.isCancellationRequested || m.getVersionId() !== version) return 0;
  return applyFileEdits(body.changes);
}

/** Sorts, merges and removes unused imports. */
export async function organizeImports(m: monaco.editor.ITextModel, mode: 'All' | 'SortAndCombine' | 'RemoveUnused' = 'All') {
  const rel = served(m);
  if (!rel) return 0;
  const changes = await request<FileCodeEdits[]>('organizeImports', { scope: { type: 'file', args: { file: tsFile(rel) } }, mode });
  return applyFileEdits(changes);
}

const SOURCE_ACTIONS: [kind: string, title: string, mode: 'All' | 'SortAndCombine' | 'RemoveUnused'][] = [
  ['source.organizeImports', 'Organize imports', 'All'],
  ['source.sortImports', 'Sort imports', 'SortAndCombine'],
  ['source.removeUnusedImports', 'Remove unused imports', 'RemoveUnused'],
];

monaco.languages.registerCodeActionProvider(
  LANGS,
  {
    async provideCodeActions(model, range, context, token) {
      const rel = served(model);
      if (!rel) return undefined;
      const file = tsFile(rel);
      const only = context.only;
      const wants = (kind: string) => !only || kind === only || kind.startsWith(`${only}.`);
      const actions: monaco.languages.CodeAction[] = [];

      if (wants('quickfix')) {
        const all = monaco.editor.getModelMarkers({ resource: model.uri, owner: OWNER });
        const seenFixAll = new Set<string>();
        for (const marker of context.markers.filter((m) => m.source === 'ts' && m.code)) {
          if (token.isCancellationRequested) break;
          const code = Number(typeof marker.code === 'object' ? marker.code.value : marker.code);
          const fixes = await quietly(() =>
            request<CodeFixAction[]>('getCodeFixes', { file, startLine: marker.startLineNumber, startOffset: marker.startColumn, endLine: marker.endLineNumber, endOffset: marker.endColumn, errorCodes: [code] }, { token }),
          );
          const imports = (fixes ?? []).filter((f) => f.fixName === 'import');
          for (const f of fixes ?? []) {
            actions.push({ title: f.description, kind: 'quickfix', diagnostics: [marker], edit: await workspaceEdit(f.changes), isPreferred: f.fixName === 'import' && imports.length === 1 });
            const sameCode = all.filter((m) => String(typeof m.code === 'object' ? m.code.value : m.code) === String(code)).length;
            if (f.fixId && f.fixAllDescription && sameCode > 1 && !seenFixAll.has(f.fixId)) {
              seenFixAll.add(f.fixId);
              actions.push({ title: f.fixAllDescription, kind: 'quickfix', diagnostics: [marker], command: { id: FIX_ALL, title: f.fixAllDescription, arguments: [rel, f.fixId] } });
            }
          }
        }
      }

      // Refactors on request or for a selection, not on every cursor move.
      const invoked = context.trigger === monaco.languages.CodeActionTriggerType.Invoke;
      if (wants('refactor') && (invoked || !range.isEmpty()) && !token.isCancellationRequested) {
        const span = { startLine: range.startLineNumber, startOffset: range.startColumn, endLine: range.endLineNumber, endOffset: range.endColumn };
        const refactors = await quietly(() =>
          request<{ name: string; actions: { name: string; description: string; kind?: string; notApplicableReason?: string }[] }[]>(
            'getApplicableRefactors',
            { file, ...span, triggerReason: invoked ? 'invoked' : 'implicit', kind: only?.startsWith('refactor') ? only : undefined },
            { token },
          ),
        );
        for (const r of refactors ?? []) {
          for (const a of r.actions) {
            if (a.notApplicableReason) continue;
            actions.push({ title: a.description, kind: a.kind ?? 'refactor', command: { id: REFACTOR, title: a.description, arguments: [rel, span, r.name, a.name] } });
          }
        }
      }

      // Source actions only when asked for (Source Action…, Shift Alt O).
      if (only && (only === 'source' || only.startsWith('source.'))) {
        for (const [kind, title, mode] of SOURCE_ACTIONS) {
          if (!wants(kind)) continue;
          const changes = await quietly(() => request<FileCodeEdits[]>('organizeImports', { scope: { type: 'file', args: { file } }, mode }, { token }));
          if (changes) actions.push({ title, kind, edit: await workspaceEdit(changes) });
        }
        if (wants('source.addMissingImports')) actions.push({ title: 'Add all missing imports', kind: 'source.addMissingImports', command: { id: ADD_MISSING, title: 'Add all missing imports', arguments: [rel] } });
      }
      return { actions, dispose() {} };
    },
  },
  { providedCodeActionKinds: ['quickfix', 'refactor', 'refactor.extract', 'refactor.rewrite', 'refactor.move', 'source', ...SOURCE_ACTIONS.map(([k]) => k), 'source.addMissingImports'] },
);

// ---- save: add missing imports --------------------------------------------------------------------------------
saveParticipants.push(async (m) => {
  if (!useApp.getState().settings?.addMissingImportsOnSave || !served(m)) return;
  // A save never waits long for this (the first request can queue behind the project load).
  const cts = new monaco.CancellationTokenSource();
  const timer = setTimeout(() => cts.cancel(), 2500);
  try {
    await Promise.race([addMissingImports(m, cts.token), new Promise((r) => setTimeout(r, 2500))]);
  } finally {
    clearTimeout(timer);
    cts.cancel();
  }
});

// ---- file renames: update imports -----------------------------------------------------------------------------
/** The import updates a rename of `from` to `to` needs, asked before the rename happens on disk. */
export async function editsForRename(from: string, to: string): Promise<FileCodeEdits[]> {
  if (!active) return [];
  return (await quietly(() => request<FileCodeEdits[]>('getEditsForFileRename', { oldFilePath: tsFile(from), newFilePath: tsFile(to) }, { timeoutMs: 20_000 }))) ?? [];
}

/** Applies them after the rename; edits to files that moved go to their new paths. */
export async function applyRenameEdits(edits: FileCodeEdits[], from: string, to: string) {
  const moved = (rel: string) => (rel === from || rel.startsWith(`${from}/`) ? to + rel.slice(from.length) : rel);
  const files = await applyFileEdits(edits, moved);
  if (files) toast(`Updated imports in ${files} file${files === 1 ? '' : 's'}`, 'success');
  updateMarkers();
}

// Dev builds: a handle for poking at the server from the browser console.
if ((import.meta as unknown as { env?: { DEV?: boolean } }).env?.DEV) (window as unknown as Record<string, unknown>).__nfTs = { request, organizeImports, applyFileEdits, state: () => ({ active, root: rootFwd, tracked: [...tracked.keys()], queued: queue.length }) };
