// Explorer state and file operations: new file and folder, rename and delete, shared by the
// tree, its context menus, the File menu and the keyboard. Deletes go to the Recycle Bin, and
// open buffers follow a rename or close with a delete.
import { create } from 'zustand';
import { api } from './api';
import { useApp, openFile, askConfirm, guard, toast, setLayout } from './store';
import { renameBuffers, dropBuffers, isDirty } from './monaco';
import { editsForRename, applyRenameEdits } from './intellisense';
import { basename, dirname } from './util';

export type EntryKind = 'file' | 'folder';

interface ExplorerState {
  mode: 'nest' | 'files';
  /** Folders open in the Files tree. */
  expanded: Record<string, boolean>;
  /** The focused row: a file, a folder, or null (the project root). */
  selected: { path: string; dir: boolean } | null;
  /** An inline name box for a new entry in `dir` ('' is the root). */
  creating: { dir: string; kind: EntryKind } | null;
  /** The path whose row is an inline rename box. */
  renaming: string | null;
  /** Bumped after every change so listings refresh before the watcher catches up. */
  version: number;
}

function savedMode(): ExplorerState['mode'] {
  try {
    return localStorage.getItem('nf.explorerMode') === 'files' ? 'files' : 'nest';
  } catch {
    return 'nest';
  }
}

export const useExplorer = create<ExplorerState>(() => ({ mode: savedMode(), expanded: {}, selected: null, creating: null, renaming: null, version: 0 }));
const set = useExplorer.setState;
const get = useExplorer.getState;

export function setExplorerMode(mode: ExplorerState['mode']) {
  set({ mode });
  try {
    localStorage.setItem('nf.explorerMode', mode);
  } catch {
    /* storage unavailable */
  }
}

export function select(path: string, dir: boolean) {
  set({ selected: { path, dir } });
}

export function toggleFolder(path: string) {
  set((s) => ({ expanded: { ...s.expanded, [path]: !s.expanded[path] } }));
}

export function collapseAll() {
  set({ expanded: {} });
}

export function refreshTree() {
  set((s) => ({ version: s.version + 1 }));
}

/** The folder new entries go in: the selected folder, the selected file's folder, or the open file's. */
export function targetDir() {
  const sel = get().selected;
  if (sel) return sel.dir ? sel.path : dirname(sel.path);
  const active = useApp.getState().active;
  return active ? dirname(active) : '';
}

function expandTo(dir: string) {
  const expanded = { ...get().expanded };
  const parts = dir ? dir.split('/') : [];
  for (let i = 1; i <= parts.length; i++) expanded[parts.slice(0, i).join('/')] = true;
  set({ expanded });
}

/** Opens an inline name box for a new file or folder; shows the Files tree if needed. */
export function startCreate(kind: EntryKind, dir = targetDir()) {
  if (!useApp.getState().model) return;
  useApp.setState({ view: 'explorer' });
  if (useApp.getState().layout.sideHidden) setLayout({ sideHidden: false });
  setExplorerMode('files');
  expandTo(dir);
  set({ creating: { dir, kind }, renaming: null });
}

export function startRename(path: string) {
  set({ renaming: path, creating: null });
}

export function cancelInline() {
  set({ creating: null, renaming: null });
}

const BAD_CHARS = /[<>:"|?*\\\u0000-\u001f]/;

/** Why `name` can't be used in `siblings`, or null when it can. */
export function nameProblem(name: string, siblings: string[], current?: string): string | null {
  const n = name.trim();
  if (!n) return 'A name is required';
  if (BAD_CHARS.test(n)) return 'Names can’t contain < > : " | ? * or \\';
  const parts = n.split('/').filter(Boolean);
  if (parts.some((p) => p === '.' || p === '..')) return 'Names can’t be . or ..';
  if (parts.some((p) => /[. ]$/.test(p))) return 'Names can’t end with a dot or a space';
  const first = parts[0]?.toLowerCase();
  const clash = first && first !== current?.toLowerCase() ? siblings.find((s) => s.toLowerCase() === first) : undefined;
  return clash ? `${clash} already exists here` : null;
}

const join = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);

/** Creates what the inline box asked for. A trailing slash makes a folder, and `a/b.ts` makes `a` too. */
export async function finishCreate(name: string) {
  const c = get().creating;
  set({ creating: null });
  if (!c) return;
  const trimmed = name.trim().replace(/^\/+/, '');
  if (!trimmed) return;
  const folder = c.kind === 'folder' || trimmed.endsWith('/');
  const path = join(c.dir, trimmed.replace(/\/+$/, ''));
  const ok = await guard(folder ? api.createDir(path) : api.createFile(path));
  if (!ok) return;
  if (folder) expandTo(path);
  else expandTo(dirname(path));
  set({ selected: { path, dir: folder } });
  refreshTree();
  // With a line, the editor takes focus, ready to type.
  if (!folder) openFile(path, 1);
}

export async function finishRename(path: string, name: string) {
  set({ renaming: null });
  const trimmed = name.trim().replace(/^\/+|\/+$/g, '');
  if (!trimmed || trimmed === basename(path)) return;
  const to = join(dirname(path), trimmed);
  // Asked before the move, while TypeScript still sees the old path; applied after it.
  const importEdits = /\.(m|c)?[jt]sx?$/.test(path) || !/\.[^/]+$/.test(path) ? await editsForRename(path, to) : [];
  const ok = await guard(api.renameFile(path, to));
  if (!ok) return;
  const sel = get().selected;
  if (sel?.path === path) set({ selected: { path: to, dir: sel.dir } });
  const expanded = get().expanded;
  if (expanded[path]) set({ expanded: { ...expanded, [path]: false, [to]: true } });
  await renameBuffers(path, to);
  if (importEdits.length) await applyRenameEdits(importEdits, path, to);
  refreshTree();
}

export async function deleteEntry(path: string, dir: boolean) {
  if (!path) return;
  const tabs = useApp.getState().tabs.filter((t) => t.path === path || t.path.startsWith(`${path}/`));
  const unsaved = tabs.filter((t) => isDirty(t.path)).length;
  const ok = await askConfirm({
    title: `Delete ${basename(path)}?`,
    message:
      `${dir ? 'This folder and everything in it' : 'This file'} will be moved to the Recycle Bin.` +
      (unsaved ? ` ${unsaved === 1 ? 'An open file has' : `${unsaved} open files have`} unsaved changes that will be lost.` : ''),
    confirmLabel: 'Move to Recycle Bin',
    danger: true,
  });
  if (!ok) return;
  const done = await guard(api.deleteFile(path));
  if (!done) return;
  dropBuffers(path);
  if (get().selected?.path === path) set({ selected: null });
  refreshTree();
  toast(`Moved ${basename(path)} to the Recycle Bin`);
}

export function copyPath(path: string, relative: boolean) {
  const root = useApp.getState().model?.root ?? '';
  const sep = root.includes('\\') ? '\\' : '/';
  const text = relative ? path : `${root}${path ? sep + path.split('/').join(sep) : ''}`;
  void navigator.clipboard?.writeText(text).then(() => toast('Copied'));
}

export function reveal(path: string) {
  void guard(api.revealPath(path));
}
