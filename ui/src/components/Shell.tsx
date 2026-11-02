import { useEffect, useMemo, useRef, useState } from 'react';
import { api, pickFolder, windowAction } from '../api';
import { Icon, Logo, Play, StopSquare } from '../icons';
import {
  useApp,
  openProject,
  closeProject,
  openFile,
  startScript,
  stopProcess,
  startDebug,
  stopDebug,
  runTests,
  typecheck,
  undo,
  redo,
  setView,
  guard,
  toast,
  type View,
} from '../store';
import { saveAll, saveFile, applyTheme } from '../monaco';
import { startCreate } from '../explorer';
import { basename, fuzzyScore, kindOf, KIND_LETTER } from '../util';

// ---- commands shared by menus and the palette ------------------------------------------------
export interface Command {
  id: string;
  label: string;
  kbd?: string;
  group: string;
  run: () => void;
  enabled?: () => boolean;
}

const hasProject = () => !!useApp.getState().model;

export function openGenerator(directory: string | null = null, module: string | null = null, schematic: 'resource' | 'service' | 'controller' | 'guard' | 'pipe' | 'interceptor' | 'filter' | 'dto' | 'gateway' | 'module' = 'resource') {
  const s = useApp.getState();
  if (!s.model) return;
  let dir = directory;
  let mod = module;
  if (!dir) {
    const root = s.model.modules.find((m) => m.name === (mod ?? s.model!.rootModules[0]));
    if (root) {
      dir = root.file.split('/').slice(0, -1).join('/');
      mod = root.name;
    } else dir = s.model.sourceRoot;
  }
  useApp.setState({ generator: { schematic, directory: dir, module: mod }, view: 'generate' });
}

export async function openProjectDialog() {
  const p = await pickFolder('Open a Nest project');
  if (p) await openProject(p);
}

export function commands(): Command[] {
  const s = useApp.getState();
  const script = s.runScript;
  const toggleTheme = async () => {
    const st = useApp.getState().settings;
    if (!st) return;
    const next = { ...st, theme: document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark' };
    useApp.setState({ settings: next });
    applyTheme(next.theme);
    await guard(api.saveSettings(next));
  };
  const views: [View, string][] = [
    ['explorer', 'Explorer'],
    ['graph', 'Module graph'],
    ['database', 'Database connections'],
    ['packages', 'Packages'],
    ['api', 'API explorer'],
    ['tests', 'Tests'],
    ['env', 'Environment'],
    ['settings', 'Settings'],
  ];
  return [
    { id: 'new', group: 'File', label: 'New project…', run: () => useApp.setState({ screen: 'wizard' }) },
    { id: 'open', group: 'File', label: 'Open project…', kbd: 'Ctrl O', run: () => void openProjectDialog() },
    { id: 'newFile', group: 'File', label: 'New file…', kbd: 'Ctrl N', enabled: hasProject, run: () => startCreate('file') },
    { id: 'newFolder', group: 'File', label: 'New folder…', enabled: hasProject, run: () => startCreate('folder') },
    { id: 'save', group: 'File', label: 'Save', kbd: 'Ctrl S', enabled: () => !!useApp.getState().active, run: () => void saveFile(useApp.getState().active!) },
    { id: 'saveAll', group: 'File', label: 'Save all', kbd: 'Ctrl Shift S', enabled: hasProject, run: () => void saveAll() },
    { id: 'close', group: 'File', label: 'Close project', enabled: hasProject, run: () => void closeProject() },
    { id: 'undo', group: 'Edit', label: 'Undo last generation', kbd: 'Ctrl Z', enabled: hasProject, run: () => void undo() },
    { id: 'redo', group: 'Edit', label: 'Redo generation', kbd: 'Ctrl Y', enabled: hasProject, run: () => void redo() },
    ...views.map(([v, label]) => ({ id: `view-${v}`, group: 'View', label, enabled: hasProject, run: () => setView(v) })),
    { id: 'panel', group: 'View', label: 'Toggle bottom panel', kbd: 'Ctrl `', enabled: hasProject, run: () => useApp.setState((x) => ({ panelOpen: !x.panelOpen })) },
    { id: 'theme', group: 'View', label: 'Switch light / dark theme', run: () => void toggleTheme() },
    { id: 'generate', group: 'Nest', label: 'Generate…', kbd: 'Ctrl Alt G', enabled: hasProject, run: () => openGenerator() },
    { id: 'gen-resource', group: 'Nest', label: 'Generate resource (CRUD)', enabled: hasProject, run: () => openGenerator(null, null, 'resource') },
    { id: 'gen-module', group: 'Nest', label: 'Generate module', enabled: hasProject, run: () => openGenerator(null, null, 'module') },
    { id: 'entity', group: 'Nest', label: 'Design entity…', enabled: hasProject, run: () => useApp.setState({ entityDesigner: {} }) },
    { id: 'graph', group: 'Nest', label: 'Open module graph', enabled: hasProject, run: () => setView('graph') },
    { id: 'db', group: 'Nest', label: 'Connect a database…', enabled: hasProject, run: () => setView('database') },
    { id: 'typecheck', group: 'Nest', label: 'Type check project (tsc)', enabled: hasProject, run: () => { useApp.setState({ panelOpen: true, panelTab: 'problems' }); void typecheck(); } },
    { id: 'rescan', group: 'Nest', label: 'Rescan project', enabled: hasProject, run: () => void guard(api.rescan()).then((m) => m && useApp.setState({ model: m })) },
    { id: 'start', group: 'Run', label: `Start ${script}`, enabled: hasProject, run: () => void startScript() },
    { id: 'stop', group: 'Run', label: 'Stop', enabled: () => !!useApp.getState().running.run, run: () => void stopProcess('run') },
    { id: 'debug', group: 'Run', label: 'Debug app', kbd: 'F5', enabled: hasProject, run: () => void startDebug('app') },
    { id: 'debugTest', group: 'Run', label: 'Debug current test file', enabled: () => !!useApp.getState().active?.endsWith('.spec.ts'), run: () => void startDebug('test', useApp.getState().active) },
    { id: 'stopDebug', group: 'Run', label: 'Stop debugging', kbd: 'Shift F5', enabled: () => !['idle', 'ended'].includes(useApp.getState().debug.state), run: () => void stopDebug() },
    { id: 'tests', group: 'Run', label: 'Run all unit tests', enabled: hasProject, run: () => { setView('tests'); void runTests(); } },
    { id: 'e2e', group: 'Run', label: 'Run e2e tests', enabled: hasProject, run: () => { setView('tests'); void runTests({ e2e: true, coverage: false }); } },
  ];
}

// ---- title bar -----------------------------------------------------------------------------------
function Menus() {
  const [open, setOpen] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const settings = useApp((s) => s.settings);
  useEffect(() => {
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(null);
    };
    window.addEventListener('mousedown', close);
    return () => window.removeEventListener('mousedown', close);
  }, []);
  const groups = ['File', 'Edit', 'View', 'Nest', 'Run'];
  const cmds = open ? commands().filter((c) => c.group === open) : [];
  return (
    <nav className="menubar" aria-label="Menu" ref={ref}>
      {groups.map((g) => (
        <div key={g} style={{ position: 'relative' }}>
          <button aria-expanded={open === g} onClick={() => setOpen(open === g ? null : g)} onMouseEnter={() => open && setOpen(g)}>
            {g}
          </button>
          {open === g && (
            <div className="menu" role="menu" style={{ left: 0 }}>
              {cmds.map((c) => (
                <button
                  key={c.id}
                  role="menuitem"
                  disabled={c.enabled ? !c.enabled() : false}
                  onClick={() => {
                    setOpen(null);
                    c.run();
                  }}
                >
                  {c.label}
                  {c.kbd && <span className="kbd mono">{c.kbd}</span>}
                </button>
              ))}
              {g === 'File' && settings?.recentProjects.length ? (
                <>
                  <hr />
                  {settings.recentProjects.slice(0, 5).map((p) => (
                    <button
                      key={p}
                      onClick={() => {
                        setOpen(null);
                        void openProject(p);
                      }}
                    >
                      <span className="faint">Recent</span> {basename(p.replace(/\\/g, '/'))}
                    </button>
                  ))}
                </>
              ) : null}
            </div>
          )}
        </div>
      ))}
    </nav>
  );
}

function RunControls() {
  const model = useApp((s) => s.model);
  const running = useApp((s) => s.running.run);
  const script = useApp((s) => s.runScript);
  if (!model) return null;
  const scripts = model.scripts.filter((s) => s.startsWith('start')).concat(model.scripts.filter((s) => !s.startsWith('start') && !s.startsWith('test') && !['lint', 'format', 'build'].includes(s)));
  return (
    <div className="run-controls">
      <select className="script mono" value={script} onChange={(e) => useApp.setState({ runScript: e.target.value })} aria-label="Script to run">
        {(scripts.length ? scripts : ['start:dev']).map((s) => (
          <option key={s} value={s}>
            {s}
          </option>
        ))}
      </select>
      <button className="icon-btn primary" aria-label={running ? 'Restart' : 'Start'} title={running ? 'Restart' : `Start ${script}`} onClick={() => void startScript()}>
        {running ? <Icon name="restart" size={15} /> : <Play size={12} />}
      </button>
      <button className="icon-btn" aria-label="Stop" title="Stop" disabled={!running} onClick={() => void stopProcess('run')}>
        <StopSquare size={11} />
      </button>
      <button className="icon-btn" aria-label="Debug" title="Debug app (F5)" onClick={() => void startDebug('app')}>
        <Icon name="bug" size={16} />
      </button>
    </div>
  );
}

function DebugToolbar() {
  const debug = useApp((s) => s.debug);
  const paused = debug.state === 'paused';
  const color = paused ? 'var(--warning)' : 'var(--success)';
  return (
    <div className="debug-toolbar" role="toolbar" aria-label="Debug controls">
      <span className="row" style={{ padding: '0 10px', fontSize: 12, color }}>
        <span className="dot" style={{ background: color }} />
        {debug.detail || debug.state}
      </span>
      <button className="icon-btn" aria-label="Continue" title="Continue (F5)" disabled={!paused} style={{ color: 'var(--success)' }} onClick={() => void api.debugCommand('resume')}>
        <Play size={14} />
      </button>
      <button className="icon-btn" aria-label="Step over" title="Step over (F10)" disabled={!paused} onClick={() => void api.debugCommand('stepOver')}>
        <Icon name="stepOver" size={16} />
      </button>
      <button className="icon-btn" aria-label="Step into" title="Step into (F11)" disabled={!paused} onClick={() => void api.debugCommand('stepInto')}>
        <Icon name="stepInto" size={16} />
      </button>
      <button className="icon-btn" aria-label="Step out" title="Step out (Shift F11)" disabled={!paused} onClick={() => void api.debugCommand('stepOut')}>
        <Icon name="stepOut" size={16} />
      </button>
      <button className="icon-btn" aria-label="Restart" title="Restart" onClick={() => void startDebug(debug.mode ?? 'app', debug.mode === 'test' ? useApp.getState().active : null)}>
        <Icon name="restart" size={15} />
      </button>
      <button className="icon-btn" aria-label="Stop" title="Stop (Shift F5)" style={{ color: 'var(--error)' }} onClick={() => void stopDebug()}>
        <StopSquare size={12} />
      </button>
    </div>
  );
}

export function TitleBar() {
  const model = useApp((s) => s.model);
  const screen = useApp((s) => s.screen);
  const debugging = useApp((s) => !['idle', 'ended'].includes(s.debug.state));
  return (
    <header className={`titlebar ${debugging ? 'debugging' : ''}`} data-tauri-drag-region>
      <div className="brand" data-tauri-drag-region>
        <Logo />
        <span data-tauri-drag-region>NestFactory</span>
        {screen === 'wizard' && <span className="faint">— New project</span>}
      </div>
      <Menus />
      <div style={{ flex: '1 1 200px', display: 'flex', justifyContent: 'center' }} data-tauri-drag-region>
        {model && !debugging && (
          <button className="search-trigger" onClick={() => useApp.setState({ palette: 'commands' })}>
            <Icon name="search" size={15} />
            <span style={{ flex: 1, textAlign: 'left' }}>{model.name} — search files, generate, run…</span>
            <span className="mono faint" style={{ fontSize: 11 }}>
              Ctrl K
            </span>
          </button>
        )}
        {debugging && <DebugToolbar />}
      </div>
      {!debugging && <RunControls />}
      <div className="window-buttons">
        <button aria-label="Minimize" onClick={() => void windowAction('minimize')}>
          <Icon name="minimize" size={14} />
        </button>
        <button aria-label="Maximize" onClick={() => void windowAction('maximize')}>
          <Icon name="maximize" size={13} />
        </button>
        <button aria-label="Close" className="close" onClick={() => void windowAction('close')}>
          <Icon name="close" size={14} />
        </button>
      </div>
    </header>
  );
}

// ---- tool rail ----------------------------------------------------------------------------------
export function ToolRail() {
  const view = useApp((s) => s.view);
  const issues = useApp((s) => s.model?.diagnostics.filter((d) => d.severity !== 'info').length ?? 0);
  const items: [View, string, string][] = [
    ['explorer', 'file', 'Explorer'],
    ['graph', 'graph', 'Module graph'],
    ['generate', 'generate', 'Generate'],
    ['database', 'database', 'Database'],
    ['packages', 'package', 'Packages'],
    ['api', 'api', 'API explorer'],
    ['tests', 'tests', 'Tests'],
    ['env', 'env', 'Environment'],
  ];
  return (
    <nav className="rail" aria-label="Tools">
      {items.map(([v, icon, label]) => (
        <button key={v} aria-label={label} title={label} aria-current={view === v} onClick={() => (v === 'generate' && !useApp.getState().generator ? openGenerator() : setView(v))}>
          <Icon name={icon} />
          {v === 'graph' && issues > 0 && <span className="badge-dot" />}
        </button>
      ))}
      <div style={{ flex: 1 }} />
      <button aria-label="Settings" title="Settings" aria-current={view === 'settings'} onClick={() => setView('settings')}>
        <Icon name="settings" />
      </button>
    </nav>
  );
}

// ---- status bar ----------------------------------------------------------------------------------
export function StatusBar({ dbLabel }: { dbLabel: string | null }) {
  const model = useApp((s) => s.model);
  const running = useApp((s) => s.running.run);
  const started = useApp((s) => s.appStarted);
  const cursor = useApp((s) => s.cursor);
  const debug = useApp((s) => s.debug);
  const problems = useApp((s) => s.problems.length);
  const view = useApp((s) => s.view);
  const lastPkg = useApp((s) => s.lastPackageCommand);
  if (!model) return null;
  const issues = model.diagnostics.filter((d) => d.severity !== 'info').length;
  const debugging = !['idle', 'ended'].includes(debug.state);
  return (
    <footer className={`statusbar ${debugging ? 'debugging' : ''}`}>
      {debugging ? (
        <>
          <span>Debugging · {debug.mode === 'test' ? 'Jest' : model.name}</span>
          <span>{debug.detail}</span>
        </>
      ) : (
        <button onClick={() => useApp.setState({ panelOpen: true, panelTab: 'run' })}>
          <span className={`dot ${running ? (started ? 'ok' : 'warn') : 'off'}`} />
          {running ? (started ? `Running · :${model.port}` : 'Starting…') : 'Stopped'}
        </button>
      )}
      {model.gitBranch && <span>{model.gitBranch}</span>}
      {view === 'generate' && <span>Nest CLI schematics · @nestjs/schematics</span>}
      {lastPkg && (
        <button className="mono" title={lastPkg} style={{ maxWidth: 360, overflow: 'hidden', textOverflow: 'ellipsis' }} onClick={() => setView('packages')}>
          Last: {lastPkg}
        </button>
      )}
      {dbLabel && (
        <button onClick={() => setView('database')}>
          <span className="dot ok" />
          {dbLabel}
        </button>
      )}
      {issues > 0 && (
        <button style={{ color: 'var(--warning)' }} onClick={() => setView('graph')}>
          {issues} graph issue{issues === 1 ? '' : 's'}
        </button>
      )}
      {problems > 0 && (
        <button style={{ color: 'var(--error)' }} onClick={() => useApp.setState({ panelOpen: true, panelTab: 'problems' })}>
          {problems} type error{problems === 1 ? '' : 's'}
        </button>
      )}
      <span style={{ marginLeft: 'auto' }}>
        {model.nestVersion ? `NestJS ${model.nestVersion}` : 'NestJS'} · TypeScript
      </span>
      <span>
        Ln {cursor.line}, Col {cursor.col}
      </span>
      <span>UTF-8 · LF</span>
    </footer>
  );
}

// ---- toasts ----------------------------------------------------------------------------------------
export function Toasts() {
  const toasts = useApp((s) => s.toasts);
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`}>
          <span style={{ flex: 1 }}>{t.text}</span>
          {t.action && (
            <button
              className="btn sm"
              onClick={() => {
                t.action!.run();
                useApp.setState((s) => ({ toasts: s.toasts.filter((x) => x.id !== t.id) }));
              }}
            >
              {t.action.label}
            </button>
          )}
          <button className="icon-btn" style={{ width: 22, height: 22 }} aria-label="Dismiss" onClick={() => useApp.setState((s) => ({ toasts: s.toasts.filter((x) => x.id !== t.id) }))}>
            <Icon name="close" size={12} />
          </button>
        </div>
      ))}
    </div>
  );
}

// ---- context menu ----------------------------------------------------------------------------------
export interface MenuItem {
  label: string;
  run?: () => void;
  separator?: boolean;
  disabled?: boolean;
  kbd?: string;
}

export function ContextMenu({ x, y, items, onClose }: { x: number; y: number; items: MenuItem[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('mousedown', close);
    window.addEventListener('keydown', esc);
    return () => {
      window.removeEventListener('mousedown', close);
      window.removeEventListener('keydown', esc);
    };
  }, [onClose]);
  const top = Math.max(8, Math.min(y, window.innerHeight - items.length * 31 - 20));
  const left = Math.min(x, window.innerWidth - 260);
  return (
    <div className="context-menu" ref={ref} style={{ left, top }}>
      <div className="menu" role="menu">
        {items.map((it, i) =>
          it.separator ? (
            <hr key={i} />
          ) : (
            <button
              key={i}
              role="menuitem"
              disabled={it.disabled}
              onClick={() => {
                onClose();
                it.run?.();
              }}
            >
              {it.label}
              {it.kbd && <span className="kbd mono">{it.kbd}</span>}
            </button>
          ),
        )}
      </div>
    </div>
  );
}

// ---- confirm dialog ----------------------------------------------------------------------------------
export function ConfirmDialog() {
  const c = useApp((s) => s.confirm);
  const okRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (c) setTimeout(() => okRef.current?.focus(), 0);
  }, [c]);
  if (!c) return null;
  const done = (ok: boolean) => {
    useApp.setState({ confirm: null });
    c.resolve(ok);
  };
  return (
    <div className="overlay" style={{ zIndex: 70 }} onMouseDown={(e) => e.target === e.currentTarget && done(false)} onKeyDown={(e) => e.key === 'Escape' && done(false)}>
      <div className="dialog small confirm" role="alertdialog" aria-labelledby="confirm-title" aria-describedby="confirm-body">
        <div className="dialog-head">
          {c.danger && (
            <span className="icon-tile">
              <Icon name="trash" size={16} />
            </span>
          )}
          <h2 id="confirm-title">{c.title}</h2>
        </div>
        <p id="confirm-body" className="muted">
          {c.message}
        </p>
        <div className="dialog-foot">
          <button className="btn lg" style={{ marginLeft: 'auto' }} onClick={() => done(false)}>
            Cancel
          </button>
          <button ref={okRef} className={`btn lg ${c.danger ? 'danger' : 'primary'}`} onClick={() => done(true)}>
            {c.confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---- command palette --------------------------------------------------------------------------------
let fileCache: { root: string; files: string[] } | null = null;

export function CommandPalette() {
  const mode = useApp((s) => s.palette);
  const model = useApp((s) => s.model);
  const [q, setQ] = useState(mode === 'files' ? '' : '');
  const [files, setFiles] = useState<string[]>([]);
  const [sel, setSel] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!mode) return;
    setQ(mode === 'commands' ? '' : '');
    setSel(0);
    setTimeout(() => inputRef.current?.focus(), 0);
    if (model && (!fileCache || fileCache.root !== model.root)) {
      api.listFiles().then((f) => {
        fileCache = { root: model.root, files: f };
        setFiles(f);
      }, () => setFiles([]));
    } else if (fileCache) setFiles(fileCache.files);
  }, [mode, model]);

  type Item = { key: string; label: string; detail?: string; kind?: string; run: () => void };
  const items: Item[] = useMemo(() => {
    if (!mode) return [];
    const query = q.startsWith('>') ? q.slice(1).trim() : q.trim();
    const out: { item: Item; score: number }[] = [];
    if (mode === 'commands') {
      for (const c of commands()) {
        if (c.enabled && !c.enabled()) continue;
        const score = fuzzyScore(query, `${c.group} ${c.label}`);
        if (score >= 0) out.push({ item: { key: c.id, label: c.label, detail: c.kbd ?? c.group, run: c.run }, score: score + 500 });
      }
    }
    if (!q.startsWith('>')) {
      for (const f of files) {
        const score = fuzzyScore(query, f);
        if (score >= 0 && query) out.push({ item: { key: `f:${f}`, label: basename(f), detail: f, kind: kindOf(f), run: () => openFile(f) }, score: score + (mode === 'files' ? 1000 : 0) });
      }
      for (const c of model?.classes ?? []) {
        const score = fuzzyScore(query, c.name);
        if (score >= 0 && query) out.push({ item: { key: `c:${c.name}:${c.file}`, label: c.name, detail: `${c.kind} · ${c.file}`, kind: c.kind, run: () => openFile(c.file, c.line) }, score: score + 200 });
      }
      if (mode === 'files' && !query) {
        for (const f of files.slice(0, 50)) out.push({ item: { key: `f:${f}`, label: basename(f), detail: f, kind: kindOf(f), run: () => openFile(f) }, score: 0 });
      }
    }
    out.sort((a, b) => b.score - a.score);
    return out.slice(0, 60).map((x) => x.item);
  }, [mode, q, files, model]);

  if (!mode) return null;
  const close = () => useApp.setState({ palette: null });
  const choose = (i: number) => {
    const it = items[i];
    if (!it) return;
    close();
    it.run();
  };
  return (
    <>
      <div style={{ position: 'fixed', inset: 0, zIndex: 59 }} onMouseDown={close} />
      <div className="palette" role="dialog" aria-label="Command palette">
        <input
          ref={inputRef}
          value={q}
          placeholder={mode === 'files' ? 'Go to file or class' : 'Type a command, file or class'}
          aria-label="Search"
          onChange={(e) => {
            setQ(e.target.value);
            setSel(0);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Escape') close();
            if (e.key === 'ArrowDown') {
              e.preventDefault();
              setSel((s) => Math.min(s + 1, items.length - 1));
            }
            if (e.key === 'ArrowUp') {
              e.preventDefault();
              setSel((s) => Math.max(s - 1, 0));
            }
            if (e.key === 'Enter') choose(sel);
          }}
        />
        <div className="items" role="listbox">
          {items.length === 0 && <div className="empty" style={{ height: 'auto', padding: 16 }}>No matches</div>}
          {items.map((it, i) => (
            <button key={it.key} className="item" role="option" aria-selected={i === sel} onMouseEnter={() => setSel(i)} onClick={() => choose(i)}>
              {it.kind ? <span className={`kind ${it.kind}`}>{KIND_LETTER[it.kind] ?? '·'}</span> : <Icon name="chevronRight" size={14} />}
              <span>{it.label}</span>
              {it.detail && (
                <span className="faint mono" style={{ marginLeft: 'auto', fontSize: 11.5, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: '55%' }}>
                  {it.detail}
                </span>
              )}
            </button>
          ))}
        </div>
      </div>
    </>
  );
}

export function useGlobalKeys() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = useApp.getState();
      const target = e.target as HTMLElement;
      const inEditor = !!target.closest?.('.monaco-editor');
      const inInput = ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName) || inEditor;
      const ctrl = e.ctrlKey || e.metaKey;
      const paused = s.debug.state === 'paused';
      const debugging = !['idle', 'ended'].includes(s.debug.state);
      if (ctrl && !e.shiftKey && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        if (s.model) useApp.setState({ palette: 'commands' });
      } else if (ctrl && e.shiftKey && e.key.toLowerCase() === 'p') {
        e.preventDefault();
        if (s.model) useApp.setState({ palette: 'commands' });
      } else if (ctrl && !e.shiftKey && e.key.toLowerCase() === 'p') {
        e.preventDefault();
        if (s.model) useApp.setState({ palette: 'files' });
      } else if (ctrl && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'n') {
        e.preventDefault();
        if (s.model) startCreate('file');
      } else if (ctrl && e.key.toLowerCase() === 'o') {
        e.preventDefault();
        void openProjectDialog();
      } else if (ctrl && e.shiftKey && e.key.toLowerCase() === 's') {
        e.preventDefault();
        void saveAll();
      } else if (ctrl && e.altKey && e.key.toLowerCase() === 'g') {
        e.preventDefault();
        openGenerator();
      } else if (ctrl && e.key === '`') {
        e.preventDefault();
        useApp.setState((x) => ({ panelOpen: !x.panelOpen }));
      } else if (ctrl && !e.shiftKey && e.key.toLowerCase() === 'z' && !inInput && s.model && !s.preview) {
        e.preventDefault();
        void undo();
      } else if (ctrl && (e.key.toLowerCase() === 'y' || (e.shiftKey && e.key.toLowerCase() === 'z')) && !inInput && s.model) {
        e.preventDefault();
        void redo();
      } else if (e.key === 'F5' && !e.shiftKey) {
        e.preventDefault();
        if (paused) void api.debugCommand('resume');
        else if (!debugging && s.model) void startDebug('app');
      } else if (e.key === 'F5' && e.shiftKey) {
        e.preventDefault();
        if (debugging) void stopDebug();
      } else if (e.key === 'F10' && paused) {
        e.preventDefault();
        void api.debugCommand('stepOver');
      } else if (e.key === 'F11' && paused) {
        e.preventDefault();
        void api.debugCommand(e.shiftKey ? 'stepOut' : 'stepInto');
      } else if (e.key === 'Escape' && s.palette) {
        useApp.setState({ palette: null });
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}

export function notifyDone(text: string) {
  toast(text, 'success');
}
