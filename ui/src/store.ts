import { create } from 'zustand';
import { api } from './api';
import type { ProjectModel } from './bindings/ProjectModel';
import type { Settings } from './bindings/Settings';
import type { Toolchain } from './bindings/Toolchain';
import type { Problem } from './bindings/Problem';
import type { TestRun } from './bindings/TestRun';
import type { Changeset } from './bindings/Changeset';
import type { Schematic } from './bindings/Schematic';
import type { Breakpoint } from './bindings/Breakpoint';
import type { ApiRequest } from './bindings/ApiRequest';
import type { LatestVersion } from './bindings/LatestVersion';
import type { PackageAction } from './bindings/PackageAction';

export type View = 'explorer' | 'graph' | 'generate' | 'database' | 'packages' | 'api' | 'tests' | 'env' | 'settings';
export type PanelTab = 'run' | 'problems' | 'tests' | 'output' | 'terminal';

export interface LogLine {
  stream: string;
  text: string;
}

export interface TerminalInfo {
  id: string;
  name: string;
  profile: string;
  exited: boolean;
}

export interface Tab {
  path: string;
  dirty: boolean;
}

export interface DebugScope {
  type: string;
  name: string;
  objectId: string;
}

export interface DebugFrame {
  function: string;
  file: string | null;
  line: number;
  column: number;
  external: boolean;
  scopes: DebugScope[];
}

export interface Toast {
  id: number;
  kind: 'info' | 'error' | 'success';
  text: string;
  action?: { label: string; run: () => void };
}

export interface Preview {
  changeset: Changeset;
  title?: string;
  onApplied?: () => void;
}

/** Widths and visibility of the explorer (left) and inspector (right), kept between runs. */
export interface Layout {
  sideWidth: number;
  sideHidden: boolean;
  inspectorWidth: number;
  inspectorHidden: boolean;
}

export const LAYOUT_DEFAULT: Layout = { sideWidth: 260, sideHidden: false, inspectorWidth: 300, inspectorHidden: false };

function loadLayout(): Layout {
  try {
    return { ...LAYOUT_DEFAULT, ...JSON.parse(localStorage.getItem('nf.layout') ?? '{}') };
  } catch {
    return LAYOUT_DEFAULT;
  }
}

export interface Confirm {
  title: string;
  message: string;
  confirmLabel: string;
  danger?: boolean;
  resolve: (ok: boolean) => void;
}

interface State {
  screen: 'welcome' | 'wizard' | 'project';
  model: ProjectModel | null;
  settings: Settings | null;
  toolchain: Toolchain | null;
  view: View;
  tabs: Tab[];
  active: string | null;
  reveal: { path: string; line: number; nonce: number } | null;
  cursor: { line: number; col: number };
  panelOpen: boolean;
  panelTab: PanelTab;
  layout: Layout;
  /** Open terminals in the panel, in tab order (their xterm instances live in terminal.ts). */
  terminals: TerminalInfo[];
  activeTerminal: string | null;
  logs: Record<string, LogLine[]>;
  running: Record<string, boolean>;
  runScript: string;
  appStarted: boolean;
  problems: Problem[];
  typechecking: boolean;
  testRun: TestRun | null;
  testsRunning: boolean;
  generator: { schematic: Schematic; directory: string | null; module: string | null } | null;
  preview: Preview | null;
  confirm: Confirm | null;
  breakpoints: Breakpoint[];
  debug: { state: string; detail: string; frames: DebugFrame[] | null; mode: 'app' | 'test' | null };
  apiDraft: ApiRequest | null;
  toasts: Toast[];
  palette: 'commands' | 'files' | null;
  entityDesigner: { directory?: string; module?: string } | null;
  busy: string | null;
  /** The package command running now (Packages view), and the last one that ran. */
  packagesRunning: string | null;
  lastPackageCommand: string | null;
  /** Latest versions from the registry, kept while the project is open. */
  latestVersions: LatestVersion[] | null;
  /** The formatter the last format used, and its error if it failed (for the status bar). */
  formatter: { name: string; version: string | null; error: string | null } | null;
  /** Window zoom factor (1 = 100%). */
  zoom: number;
}

export const useApp = create<State>(() => ({
  screen: 'welcome',
  model: null,
  settings: null,
  toolchain: null,
  view: 'explorer',
  tabs: [],
  active: null,
  reveal: null,
  cursor: { line: 1, col: 1 },
  panelOpen: true,
  panelTab: 'run',
  layout: loadLayout(),
  terminals: [],
  activeTerminal: null,
  logs: {},
  running: {},
  runScript: 'start:dev',
  appStarted: false,
  problems: [],
  typechecking: false,
  testRun: null,
  testsRunning: false,
  generator: null,
  preview: null,
  confirm: null,
  breakpoints: [],
  debug: { state: 'idle', detail: '', frames: null, mode: null },
  apiDraft: null,
  toasts: [],
  palette: null,
  entityDesigner: null,
  busy: null,
  packagesRunning: null,
  lastPackageCommand: null,
  latestVersions: null,
  formatter: null,
  zoom: 1,
}));

const set = useApp.setState;
const get = useApp.getState;

let toastId = 1;
export function toast(text: string, kind: Toast['kind'] = 'info', action?: Toast['action']) {
  const id = toastId++;
  set((s) => ({ toasts: [...s.toasts, { id, kind, text, action }] }));
  setTimeout(() => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })), kind === 'error' ? 9000 : 5000);
}

export function errorText(e: unknown) {
  return e instanceof Error ? e.message : String(e);
}

export async function guard<T>(p: Promise<T>): Promise<T | undefined> {
  try {
    return await p;
  } catch (e) {
    toast(errorText(e), 'error');
    return undefined;
  }
}

export async function openProject(path: string) {
  set({ busy: 'Opening project…' });
  try {
    const model = await api.openProject(path);
    const settings = await api.getSettings();
    set({ model, settings, screen: 'project', view: 'explorer', tabs: [], active: null, problems: [], testRun: null, logs: {}, running: {}, appStarted: false, breakpoints: loadBreakpoints(model.root), latestVersions: null, lastPackageCommand: null, formatter: null });
    const main = model.files.find((f) => f.kind === 'main');
    const firstController = model.classes.find((c) => c.kind === 'controller');
    if (firstController) openFile(firstController.file, firstController.line);
    else if (main) openFile(main.path);
  } catch (e) {
    toast(errorText(e), 'error');
  } finally {
    set({ busy: null });
  }
}

export async function closeProject() {
  await guard(api.closeProject());
  set({ screen: 'welcome', model: null, tabs: [], active: null });
}

export function openFile(path: string, line?: number) {
  set((s) => {
    const tabs = s.tabs.some((t) => t.path === path) ? s.tabs : [...s.tabs, { path, dirty: false }];
    return {
      tabs,
      active: path,
      view: s.view === 'explorer' || s.view === 'tests' ? s.view : 'explorer',
      reveal: line ? { path, line, nonce: Date.now() } : s.reveal,
    };
  });
}

export function closeTab(path: string) {
  set((s) => {
    const idx = s.tabs.findIndex((t) => t.path === path);
    const tabs = s.tabs.filter((t) => t.path !== path);
    const active = s.active === path ? (tabs[Math.min(idx, tabs.length - 1)]?.path ?? null) : s.active;
    return { tabs, active };
  });
}

export function setDirty(path: string, dirty: boolean) {
  set((s) => ({ tabs: s.tabs.map((t) => (t.path === path && t.dirty !== dirty ? { ...t, dirty } : t)) }));
}

const MAX_LOG = 3000;
export function appendLog(id: string, line: LogLine) {
  set((s) => {
    const cur = s.logs[id] ?? [];
    const next = cur.length >= MAX_LOG ? [...cur.slice(cur.length - MAX_LOG + 1), line] : [...cur, line];
    return { logs: { ...s.logs, [id]: next } };
  });
}

export function clearLog(id: string) {
  set((s) => ({ logs: { ...s.logs, [id]: [] } }));
}

/** An in-app yes/no dialog; resolves false when dismissed. */
export function askConfirm(opts: Omit<Confirm, 'resolve'>): Promise<boolean> {
  get().confirm?.resolve(false);
  return new Promise((resolve) => set({ confirm: { ...opts, resolve } }));
}

export function showPreview(changeset: Changeset, opts: Omit<Preview, 'changeset'> = {}) {
  set({ preview: { changeset, ...opts } });
}

export async function applyChangeset(cs: Changeset, onApplied?: () => void) {
  try {
    const res = await api.applyChangeset(cs.id);
    set({ model: res.model, preview: null, generator: null });
    const n = res.changeset.files.length;
    toast(`${res.changeset.label} · wrote ${n} change${n === 1 ? '' : 's'}`, 'success', { label: 'Undo', run: () => undo() });
    if (res.changeset.install.length || res.changeset.installDev.length || res.changeset.runInstall) {
      set({ panelOpen: true, panelTab: 'output' });
      toast('Installing packages…');
    }
    onApplied?.();
    return true;
  } catch (e) {
    toast(errorText(e), 'error');
    return false;
  }
}

export async function undo() {
  try {
    const r = await api.undo();
    toast(`Undid: ${r.label}`, 'info', { label: 'Redo', run: () => redo() });
  } catch (e) {
    toast(errorText(e), 'error');
  }
}

export async function redo() {
  try {
    const r = await api.redo();
    toast(`Redid: ${r.label}`);
  } catch (e) {
    toast(errorText(e), 'error');
  }
}

export async function startScript(script?: string) {
  const s = script ?? get().runScript;
  set({ runScript: s, panelOpen: true, panelTab: 'run', appStarted: false });
  clearLog('run');
  await guard(api.runScript(s));
}

export async function stopProcess(id: string) {
  await guard(api.stopProcess(id));
}

export async function runTests(opts: { e2e?: boolean; file?: string | null; name?: string | null; coverage?: boolean } = {}) {
  set({ testsRunning: true });
  clearLog('tests');
  try {
    const run = await api.runTests({ coverage: true, ...opts });
    set({ testRun: run });
    if (run.error) toast('Jest reported an error. See Tests output.', 'error');
    else toast(`${run.passed} passed${run.failed ? `, ${run.failed} failed` : ''}`, run.failed ? 'error' : 'success');
  } catch (e) {
    toast(errorText(e), 'error');
  } finally {
    set({ testsRunning: false });
  }
}

let typecheckTimer: number | undefined;
export function scheduleTypecheck(delay = 1200) {
  window.clearTimeout(typecheckTimer);
  typecheckTimer = window.setTimeout(() => void typecheck(), delay);
}

export async function typecheck() {
  if (get().typechecking) return;
  set({ typechecking: true });
  try {
    const problems = await api.typecheck();
    set({ problems });
  } catch (e) {
    toast(errorText(e), 'error');
  } finally {
    set({ typechecking: false });
  }
}

function bpKey(root: string) {
  return `nf.breakpoints.${root}`;
}

function loadBreakpoints(root: string): Breakpoint[] {
  try {
    return JSON.parse(localStorage.getItem(bpKey(root)) ?? '[]');
  } catch {
    return [];
  }
}

export function toggleBreakpoint(file: string, line: number) {
  const s = get();
  const exists = s.breakpoints.some((b) => b.file === file && b.line === line);
  const breakpoints = exists ? s.breakpoints.filter((b) => !(b.file === file && b.line === line)) : [...s.breakpoints, { file, line }];
  set({ breakpoints });
  try {
    if (s.model) localStorage.setItem(bpKey(s.model.root), JSON.stringify(breakpoints));
  } catch {
    /* storage unavailable */
  }
  if (s.debug.state !== 'idle' && s.debug.state !== 'ended') void api.debugSetBreakpoints(breakpoints);
}

export async function startDebug(mode: 'app' | 'test', file: string | null = null) {
  set({ debug: { state: 'starting', detail: 'Starting', frames: null, mode }, panelOpen: true, panelTab: 'run' });
  clearLog('debug');
  await guard(api.debugStart(mode, file, get().breakpoints));
}

export async function stopDebug() {
  await guard(api.debugStop());
  set({ debug: { state: 'idle', detail: '', frames: null, mode: null } });
}

/** Starts a package command; output streams as process `packages`, and `task-done` ends it. */
export async function runPackages(action: PackageAction, packages: string[], dev = false, devPackages: string[] = []) {
  clearLog('packages');
  try {
    const lines = await api.pkgRun(action, packages, dev, devPackages);
    set({ packagesRunning: lines.join(' && '), lastPackageCommand: lines.join(' && ') });
    return true;
  } catch (e) {
    toast(errorText(e), 'error');
    return false;
  }
}

/** Changes the side panels; `persist` false while dragging, saved on release. */
export function setLayout(patch: Partial<Layout>, persist = true) {
  set((s) => ({ layout: { ...s.layout, ...patch } }));
  if (!persist) return;
  try {
    localStorage.setItem('nf.layout', JSON.stringify(get().layout));
  } catch {
    /* storage unavailable */
  }
}

export function toggleSidebar() {
  setLayout({ sideHidden: !get().layout.sideHidden });
}

export function toggleInspector() {
  setLayout({ inspectorHidden: !get().layout.inspectorHidden });
}

export function setView(view: View) {
  set({ view });
}
