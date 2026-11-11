// Terminals in the bottom panel: xterm.js on real shells (service/pty.rs). Each terminal keeps its
// xterm instance and element for its whole life. The panel borrows the active one's element and
// the others wait in an off-screen parking lot, so output keeps arriving while they're hidden.
import { Terminal, type ITheme } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';
import { api, on, openUrl } from './api';
import { useApp, toast, errorText } from './store';
import { zoomKey } from './zoom';
import type { TerminalProfile } from './bindings/TerminalProfile';

interface Instance {
  term: Terminal;
  fit: FitAddon;
  el: HTMLDivElement;
  spawned: boolean;
  exited: boolean;
  /** Keystrokes waiting for the write in flight, so they reach the shell in order. */
  outbox: string;
  sending: boolean;
}

const instances = new Map<string, Instance>();
let seq = 0;
const FONT = "'Geist Mono Variable', 'Geist Mono', 'Cascadia Mono', Consolas, monospace";

let parking: HTMLDivElement | null = null;
function parkingLot() {
  if (!parking) {
    parking = document.createElement('div');
    parking.setAttribute('aria-hidden', 'true');
    parking.style.cssText = 'position:fixed;left:-10000px;top:0;width:800px;height:400px;overflow:hidden;';
    document.body.appendChild(parking);
  }
  return parking;
}

let profilesP: Promise<TerminalProfile[]> | null = null;
/** The shells on this machine, the default first; asked once. */
export function terminalProfiles() {
  profilesP ??= api.terminalProfiles().catch((e) => {
    profilesP = null;
    throw e;
  });
  return profilesP;
}

// ---- look ------------------------------------------------------------------------------------
const DARK_ANSI = {
  black: '#2a2c31',
  red: '#f2675f',
  green: '#3fd2be',
  yellow: '#f2b44c',
  blue: '#6cc9f0',
  magenta: '#c9a7ff',
  cyan: '#5fd0d0',
  white: '#d4d2cd',
  brightBlack: '#82858d',
  brightRed: '#ff8a82',
  brightGreen: '#72e6d5',
  brightYellow: '#ffd27a',
  brightBlue: '#97dcf7',
  brightMagenta: '#dcc6ff',
  brightCyan: '#8ee6e6',
  brightWhite: '#f6f4f0',
};
const LIGHT_ANSI = {
  black: '#1a1b1e',
  red: '#b8372f',
  green: '#0a7266',
  yellow: '#8a5700',
  blue: '#1b65a0',
  magenta: '#6f45c8',
  cyan: '#0e7490',
  white: '#6b6e75',
  brightBlack: '#50535a',
  brightRed: '#d0443b',
  brightGreen: '#0d8a7c',
  brightYellow: '#a66a00',
  brightBlue: '#2479bd',
  brightMagenta: '#8459d8',
  brightCyan: '#1590a8',
  brightWhite: '#8c8f96',
};

function theme(): ITheme {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string) => css.getPropertyValue(name).trim();
  const dark = document.documentElement.dataset.theme !== 'light';
  return {
    background: v('--bg-panel'),
    foreground: v('--text'),
    cursor: v('--ember'),
    cursorAccent: v('--bg-panel'),
    selectionBackground: dark ? '#3b3d45' : '#cdd7e4',
    scrollbarSliderBackground: dark ? '#2a2c3199' : '#d3d0c899',
    ...(dark ? DARK_ANSI : LIGHT_ANSI),
  };
}

const fontSize = () => useApp.getState().settings?.editorFontSize ?? 13;

new MutationObserver(() => {
  const t = theme();
  for (const i of instances.values()) i.term.options.theme = t;
}).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });

useApp.subscribe((s, prev) => {
  if (s.settings?.editorFontSize !== prev.settings?.editorFontSize) {
    for (const i of instances.values()) i.term.options.fontSize = fontSize();
    if (s.activeTerminal) fitTerminal(s.activeTerminal);
  }
  // Shells belong to the project they started in; the backend has already ended them.
  if (s.model?.root !== prev.model?.root) for (const id of [...instances.keys()]) closeTerminal(id, false);
});

// ---- backend events --------------------------------------------------------------------------
on('pty-output', (p: { id: string; data: string }) => instances.get(p.id)?.term.write(p.data));

on('pty-exit', (p: { id: string; code: number | null }) => {
  const inst = instances.get(p.id);
  if (!inst) return;
  inst.exited = true;
  // `exit` in the shell closes it, as in VS Code; a failure stays up so it can be read.
  if (p.code === 0) {
    closeTerminal(p.id, false);
    return;
  }
  inst.term.write(`\r\n\x1b[90m[The process exited with code ${p.code ?? '?'}. Press any key to close the terminal.]\x1b[0m\r\n`);
  setInfo(p.id, { exited: true });
});

function setInfo(id: string, patch: { exited?: boolean }) {
  useApp.setState((s) => ({ terminals: s.terminals.map((t) => (t.id === id ? { ...t, ...patch } : t)) }));
}

// ---- input -------------------------------------------------------------------------------------
function send(id: string, inst: Instance, data: string) {
  inst.outbox += data;
  if (inst.sending) return;
  inst.sending = true;
  void (async () => {
    while (inst.outbox && !inst.exited) {
      const chunk = inst.outbox;
      inst.outbox = '';
      try {
        await api.ptyWrite(id, chunk);
      } catch {
        inst.outbox = '';
      }
    }
    inst.sending = false;
  })();
}

/** Copy and paste as in VS Code on Windows, and app shortcuts the shell shouldn't swallow. */
function keys(term: Terminal, e: KeyboardEvent): boolean {
  if (e.type !== 'keydown') return true;
  const ctrl = e.ctrlKey || e.metaKey;
  const key = e.key.toLowerCase();
  if (ctrl && !e.altKey && key === 'c' && (e.shiftKey || term.hasSelection())) {
    const text = term.getSelection();
    if (text) void navigator.clipboard?.writeText(text);
    term.clearSelection();
    e.preventDefault();
    return false;
  }
  // Let the browser fire a paste event; xterm turns it into input (bracketed when asked).
  if (ctrl && !e.altKey && key === 'v') return false;
  if (ctrl && !e.altKey && (key === 'p' || key === 'b' || key === 'j' || e.code === 'Backquote')) return false;
  if (zoomKey(e)) return false;
  return true;
}

function makeInstance(id: string): Instance {
  const term = new Terminal({
    fontFamily: FONT,
    fontSize: fontSize(),
    lineHeight: 1.2,
    cursorBlink: true,
    scrollback: 5000,
    theme: theme(),
    allowProposedApi: false,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.loadAddon(new WebLinksAddon((_e, uri) => void openUrl(uri)));
  const el = document.createElement('div');
  el.className = 'xterm-wrap';
  parkingLot().appendChild(el);
  term.open(el);
  const inst: Instance = { term, fit, el, spawned: false, exited: false, outbox: '', sending: false };
  term.onData((data) => {
    if (inst.exited) closeTerminal(id, false);
    else if (inst.spawned) send(id, inst, data);
  });
  term.attachCustomKeyEventHandler((e) => keys(term, e));
  return inst;
}

const frame = () => new Promise((r) => requestAnimationFrame(() => r(null)));

// ---- lifecycle --------------------------------------------------------------------------------------
/** Opens a terminal with `profileId` (or the default shell) and shows it. */
export async function createTerminal(profileId?: string) {
  let profiles: TerminalProfile[];
  try {
    profiles = await terminalProfiles();
  } catch (e) {
    toast(errorText(e), 'error');
    return;
  }
  const want = profileId ?? useApp.getState().settings?.terminalProfile;
  const p = profiles.find((x) => x.id === want) ?? profiles[0];
  if (!p) {
    toast('No shell was found on this machine', 'error');
    return;
  }
  await document.fonts?.load(`${fontSize()}px 'Geist Mono Variable'`).catch(() => {});
  const id = `t${++seq}`;
  const inst = makeInstance(id);
  instances.set(id, inst);
  useApp.setState((s) => ({ terminals: [...s.terminals, { id, name: p.name, profile: p.id, exited: false }], activeTerminal: id, panelOpen: true, panelTab: 'terminal', view: 'explorer' }));
  // The panel mounts and fits it to its size before the shell starts.
  await frame();
  await frame();
  const { cols, rows } = inst.term;
  try {
    await api.ptySpawn(id, p.id, cols, rows);
    inst.spawned = true;
    if (inst.term.cols !== cols || inst.term.rows !== rows) void api.ptyResize(id, inst.term.cols, inst.term.rows).catch(() => {});
  } catch (e) {
    inst.exited = true;
    inst.term.write(`\x1b[31m${errorText(e)}\x1b[0m\r\n`);
    setInfo(id, { exited: true });
  }
}

let autoStarting = false;
/** The first time the Terminal tab shows, it starts a shell, as VS Code does. */
export async function ensureTerminal() {
  if (autoStarting || useApp.getState().terminals.length) return;
  autoStarting = true;
  try {
    await createTerminal();
  } finally {
    autoStarting = false;
  }
}

export function closeTerminal(id: string, kill = true) {
  const inst = instances.get(id);
  if (inst) {
    instances.delete(id);
    if (kill && !inst.exited) void api.ptyKill(id).catch(() => {});
    inst.term.dispose();
    inst.el.remove();
  }
  useApp.setState((s) => {
    const idx = s.terminals.findIndex((t) => t.id === id);
    const terminals = s.terminals.filter((t) => t.id !== id);
    return { terminals, activeTerminal: s.activeTerminal === id ? (terminals[Math.min(idx, terminals.length - 1)]?.id ?? null) : s.activeTerminal };
  });
}

/** Moves terminal `id` into `host` (parking any other there) and fits it. */
export function attachTerminal(id: string, host: HTMLElement) {
  const inst = instances.get(id);
  if (!inst) return;
  for (const other of instances.values()) if (other !== inst && other.el.parentElement === host) parkingLot().appendChild(other.el);
  if (inst.el.parentElement !== host) host.appendChild(inst.el);
  fitTerminal(id);
}

export function parkTerminals(host: HTMLElement) {
  for (const inst of instances.values()) if (inst.el.parentElement === host) parkingLot().appendChild(inst.el);
}

/** Fits the terminal to its container and tells the shell its new size. */
export function fitTerminal(id: string) {
  const inst = instances.get(id);
  if (!inst || inst.el.parentElement === parking) return;
  const dims = inst.fit.proposeDimensions();
  if (!dims || !dims.cols || !dims.rows || !Number.isFinite(dims.cols)) return;
  if (dims.cols === inst.term.cols && dims.rows === inst.term.rows) return;
  inst.term.resize(dims.cols, dims.rows);
  if (inst.spawned && !inst.exited) void api.ptyResize(id, dims.cols, dims.rows).catch(() => {});
}

export function focusTerminal(id: string) {
  instances.get(id)?.term.focus();
}

export function clearTerminal(id: string) {
  instances.get(id)?.term.clear();
}
