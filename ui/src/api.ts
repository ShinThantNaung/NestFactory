// The UI's only door to the Rust backend. In the desktop app calls go through Tauri's
// `backend` command; in a browser (development) they go to the dev bridge on localhost.
// Types come from ts-rs bindings generated from the Rust structs.

import type { ProjectModel } from './bindings/ProjectModel';
import type { Changeset } from './bindings/Changeset';
import type { GenerateOptions } from './bindings/GenerateOptions';
import type { FixAction } from './bindings/FixAction';
import type { Settings } from './bindings/Settings';
import type { DirEntry } from './bindings/DirEntry';
import type { DbConfig } from './bindings/DbConfig';
import type { DbPlan } from './bindings/DbPlan';
import type { DbProfile } from './bindings/DbProfile';
import type { DbTestResult } from './bindings/DbTestResult';
import type { DockerStatus } from './bindings/DockerStatus';
import type { TestRun } from './bindings/TestRun';
import type { Problem } from './bindings/Problem';
import type { EnvReport } from './bindings/EnvReport';
import type { Endpoint } from './bindings/Endpoint';
import type { ApiRequest } from './bindings/ApiRequest';
import type { ApiResponse } from './bindings/ApiResponse';
import type { HistoryEntry } from './bindings/HistoryEntry';
import type { WizardOptions } from './bindings/WizardOptions';
import type { WizardPlan } from './bindings/WizardPlan';
import type { Toolchain } from './bindings/Toolchain';
import type { HistoryItem } from './bindings/HistoryItem';
import type { EntityOptions } from './bindings/EntityOptions';
import type { Breakpoint } from './bindings/Breakpoint';
import type { Variable } from './bindings/Variable';
import type { RecentProject } from './bindings/RecentProject';
import type { PackageList } from './bindings/PackageList';
import type { LatestVersion } from './bindings/LatestVersion';
import type { SearchHit } from './bindings/SearchHit';
import type { CatalogEntry } from './bindings/CatalogEntry';
import type { PackageAction } from './bindings/PackageAction';
import type { PackagePlan } from './bindings/PackagePlan';
import type { PackageDetails } from './bindings/PackageDetails';
import type { FormatResult } from './bindings/FormatResult';

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

function bridgeConfig() {
  const params = new URLSearchParams(location.search);
  const port = params.get('bridge');
  const token = params.get('token');
  if (port && token) {
    try {
      sessionStorage.setItem('nf.bridge', JSON.stringify({ port, token }));
    } catch {
      /* storage unavailable */
    }
    return { port, token };
  }
  try {
    const saved = sessionStorage.getItem('nf.bridge');
    if (saved) return JSON.parse(saved) as { port: string; token: string };
  } catch {
    /* storage unavailable */
  }
  return { port: '1421', token: '' };
}

const bridge = isTauri ? null : bridgeConfig();

export async function call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (isTauri) {
    const { invoke } = await import('@tauri-apps/api/core');
    try {
      return await invoke<T>('backend', { cmd, args });
    } catch (e) {
      throw new Error(typeof e === 'string' ? e : String(e));
    }
  }
  const res = await fetch(`http://127.0.0.1:${bridge!.port}/invoke`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-bridge-token': bridge!.token },
    body: JSON.stringify({ cmd, args }),
  });
  if (res.status === 401) throw new Error('The dev bridge rejected the token. Open the URL the bridge printed.');
  const json = (await res.json()) as { ok: boolean; data?: T; error?: string };
  if (!json.ok) throw new Error(json.error ?? 'Unknown error');
  return json.data as T;
}

type Handler = (payload: any) => void;
const handlers = new Map<string, Set<Handler>>();
let source: EventSource | null = null;

function dispatchEvent(event: string, payload: unknown) {
  handlers.get(event)?.forEach((h) => h(payload));
}

/** Subscribes to a backend event; returns an unsubscribe function. */
export function on(event: string, handler: Handler): () => void {
  if (!handlers.has(event)) {
    handlers.set(event, new Set());
    if (isTauri) {
      import('@tauri-apps/api/event').then(({ listen }) => listen(event, (e) => dispatchEvent(event, e.payload)));
    } else {
      if (!source) source = new EventSource(`http://127.0.0.1:${bridge!.port}/events?token=${bridge!.token}`);
      source.addEventListener(event, (e) => dispatchEvent(event, JSON.parse((e as MessageEvent).data)));
    }
  }
  handlers.get(event)!.add(handler);
  return () => handlers.get(event)?.delete(handler);
}

export async function pickFolder(title: string): Promise<string | null> {
  if (isTauri) {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const r = await open({ directory: true, title });
    return typeof r === 'string' ? r : null;
  }
  return window.prompt(`${title} (full path)`);
}

export async function openUrl(url: string) {
  if (isTauri) {
    const { openUrl: open } = await import('@tauri-apps/plugin-opener');
    await open(url);
  } else {
    window.open(url, '_blank', 'noopener');
  }
}

export async function windowAction(action: 'minimize' | 'maximize' | 'close' | 'drag') {
  if (!isTauri) return;
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  const w = getCurrentWindow();
  if (action === 'minimize') await w.minimize();
  if (action === 'maximize') await w.toggleMaximize();
  if (action === 'close') await w.close();
  if (action === 'drag') await w.startDragging();
}

export async function launchProject(): Promise<string | null> {
  if (!isTauri) return new URLSearchParams(location.search).get('open');
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<string | null>('launch_project');
}

export interface ApplyResult {
  changeset: Changeset;
  model: ProjectModel;
}

export const api = {
  getSettings: () => call<Settings>('getSettings'),
  saveSettings: (settings: Settings) => call<Settings>('saveSettings', { settings }),
  toolchain: () => call<Toolchain>('toolchain'),
  recentProjects: () => call<RecentProject[]>('recentProjects'),
  /** Removes one recent project, or the whole list when `path` is null. */
  removeRecent: (path: string | null) => call<RecentProject[]>('removeRecent', { path }),

  openProject: (path: string) => call<ProjectModel>('openProject', { path }),
  closeProject: () => call<boolean>('closeProject'),
  rescan: () => call<ProjectModel>('rescan'),
  listDir: (path: string) => call<DirEntry[]>('listDir', { path }),
  listFiles: () => call<string[]>('listFiles'),
  readFile: (path: string) => call<{ content: string }>('readFile', { path }).then((r) => r.content),
  writeFile: (path: string, content: string) => call<boolean>('writeFile', { path, content }),
  /** Creates an empty file (and missing parent folders); fails if it exists. */
  createFile: (path: string) => call<boolean>('createFile', { path }),
  createDir: (path: string) => call<boolean>('createDir', { path }),
  /** Moves a file or folder to the Recycle Bin. */
  deleteFile: (path: string) => call<boolean>('deleteFile', { path }),
  renameFile: (from: string, to: string) => call<boolean>('renameFile', { from, to }),
  /** Formats `text` as `path` with the project's Prettier; `text: null` when it has none or skips the file. */
  formatFile: (path: string, text: string) => call<FormatResult>('formatFile', { path, text }),
  /** Shows the file or folder selected in Explorer (Finder on macOS). */
  revealPath: (path: string) => call<boolean>('revealPath', { path }),

  previewGenerate: (options: GenerateOptions) => call<Changeset>('previewGenerate', { options }),
  previewFix: (action: FixAction) => call<Changeset>('previewFix', { action }),
  previewWire: (provider: string, module: string) => call<Changeset>('previewWire', { provider, module }),
  previewEntity: (options: EntityOptions) => call<Changeset>('previewEntity', { options }),
  previewSyncDto: (entity: string) => call<Changeset>('previewSyncDto', { entity }),
  applyChangeset: (id: string) => call<ApplyResult>('applyChangeset', { id }),
  discardChangeset: (id: string) => call<boolean>('discardChangeset', { id }),
  undo: () => call<{ label: string }>('undo'),
  redo: () => call<{ label: string }>('redo'),
  history: () => call<HistoryItem[]>('history'),

  dbProfiles: () => call<DbProfile[]>('dbProfiles'),
  dbPassword: (name: string) => call<string | null>('dbPassword', { name }),
  dbPlan: (config: DbConfig) => call<DbPlan>('dbPlan', { config }),
  dbApply: (config: DbConfig, changesetId: string | null) => call<DbTestResult>('dbApply', { config, changesetId }),
  dbSaveProfile: (config: DbConfig) => call<DbProfile[]>('dbSaveProfile', { config }),
  dbDeleteProfile: (name: string) => call<DbProfile[]>('dbDeleteProfile', { name }),
  dbTest: (config: DbConfig) => call<DbTestResult>('dbTest', { config }),
  dockerStatus: (container: string) => call<DockerStatus>('dockerStatus', { container }),
  dockerStart: (service: string) => call<boolean>('dockerStart', { service }),
  dockerStop: (service: string) => call<boolean>('dockerStop', { service }),

  runScript: (script: string) => call<number>('runScript', { script }),
  stopProcess: (id: string) => call<boolean>('stopProcess', { id }),
  processes: () => call<string[]>('processes'),
  runCommand: (line: string) => call<number>('runCommand', { line }),
  runTests: (opts: { e2e?: boolean; file?: string | null; name?: string | null; coverage?: boolean }) => call<TestRun>('runTests', opts),
  typecheck: () => call<Problem[]>('typecheck'),

  pkgList: () => call<PackageList>('pkgList'),
  /** Asks the registry; slow-ish (one request per dependency, eight at a time). */
  pkgLatest: () => call<LatestVersion[]>('pkgLatest'),
  pkgSearch: (query: string) => call<SearchHit[]>('pkgSearch', { query }),
  pkgCatalog: () => call<CatalogEntry[]>('pkgCatalog'),
  /** Resolves versions for the project's Nest and finds missing peers; writes nothing. */
  pkgPlan: (action: PackageAction, packages: string[], dev = false) => call<PackagePlan>('pkgPlan', { action, packages, dev }),
  /** Registry details, compatibility checks and wiring steps for one package. */
  pkgDetails: (name: string) => call<PackageDetails>('pkgDetails', { name }),
  /** The command lines a run would use; no network. */
  pkgCommand: (action: PackageAction, packages: string[], dev = false, devPackages: string[] = []) => call<string[]>('pkgCommand', { action, packages, dev, devPackages }),
  /** Starts the commands in the background and returns them. Output streams as process `packages`;
   * `task-done` with id `packages` follows, then `project-updated`. */
  pkgRun: (action: PackageAction, packages: string[], dev = false, devPackages: string[] = []) => call<string[]>('pkgRun', { action, packages, dev, devPackages }),
  /** A changeset wiring the package into the root module (steps: `module`, `example`). */
  pkgWire: (name: string, steps: string[]) => call<Changeset>('pkgWire', { name, steps }),
  /** A changeset taking the package's wiring out, or null when there is none. */
  pkgUnwire: (name: string) => call<Changeset | null>('pkgUnwire', { name }),

  envReport: () => call<EnvReport>('envReport'),
  envSave: (file: string, updates: [string, string | null][]) => call<EnvReport>('envSave', { file, updates }),

  apiEndpoints: () => call<Endpoint[]>('apiEndpoints'),
  apiSend: (request: ApiRequest) => call<ApiResponse>('apiSend', { request }),
  apiSaved: () => call<ApiRequest[]>('apiSaved'),
  apiHistory: () => call<HistoryEntry[]>('apiHistory'),
  apiSave: (request: ApiRequest) => call<ApiRequest[]>('apiSave', { request }),
  apiDelete: (id: string) => call<ApiRequest[]>('apiDelete', { id }),

  wizardPlan: (options: WizardOptions) => call<WizardPlan>('wizardPlan', { options }),
  wizardRun: (options: WizardOptions) => call<string>('wizardRun', { options }),

  debugStart: (mode: 'app' | 'test', file: string | null, breakpoints: Breakpoint[]) => call<boolean>('debugStart', { mode, file, breakpoints }),
  debugSetBreakpoints: (breakpoints: Breakpoint[]) => call<boolean>('debugSetBreakpoints', { breakpoints }),
  debugCommand: (command: 'resume' | 'stepOver' | 'stepInto' | 'stepOut' | 'pause') => call<boolean>('debugCommand', { command }),
  debugProperties: (objectId: string) => call<Variable[]>('debugProperties', { objectId }),
  debugStop: () => call<boolean>('debugStop'),
};
