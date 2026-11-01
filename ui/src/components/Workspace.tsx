import { useEffect, useMemo, useRef, useState } from 'react';
import { api, openUrl } from '../api';
import { Icon } from '../icons';
import { useApp, openFile, setView, typecheck, runTests, clearLog, guard, toast, showPreview, type PanelTab } from '../store';
import { ansiToHtml, basename, dirname, kindOf, KIND_LETTER, moduleForFile, stripAnsi } from '../util';
import { EditorArea } from './Editor';
import { ContextMenu, openGenerator, type MenuItem } from './Shell';
import type { DirEntry } from '../bindings/DirEntry';
import type { ModuleInfo } from '../bindings/ModuleInfo';
import type { ClassInfo } from '../bindings/ClassInfo';

const KIND_ORDER = ['module', 'controller', 'service', 'gateway', 'resolver', 'entity', 'dto', 'guard', 'pipe', 'interceptor', 'filter', 'middleware', 'other', 'main', 'spec'];

export function moduleMenu(m: ModuleInfo): MenuItem[] {
  const dir = dirname(m.file);
  const gen = (s: Parameters<typeof openGenerator>[2]) => () => openGenerator(dir, m.name, s);
  return [
    { label: `Generate resource in ${m.name}…`, run: gen('resource') },
    { label: 'Generate controller…', run: gen('controller') },
    { label: 'Generate service…', run: gen('service') },
    { label: 'Generate guard / pipe / interceptor…', run: gen('guard') },
    { label: 'Generate DTO…', run: gen('dto') },
    { label: 'Generate gateway…', run: gen('gateway') },
    { label: 'Generate submodule…', run: gen('module') },
    { separator: true, label: '' },
    { label: 'Design entity…', run: () => useApp.setState({ entityDesigner: { directory: dir, module: m.name } }) },
    { label: 'Open in module graph', run: () => setView('graph') },
    { label: `Open ${basename(m.file)}`, run: () => openFile(m.file, m.line) },
  ];
}

// ---- explorer ---------------------------------------------------------------------------------
function NestTree() {
  const model = useApp((s) => s.model)!;
  const active = useApp((s) => s.active);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const issuesByModule = useMemo(() => {
    const map: Record<string, string> = {};
    for (const d of model.diagnostics) {
      if (d.severity === 'info') continue;
      for (const m of d.modules) map[m] = d.code === 'circularImport' ? 'cycle' : d.code === 'missingProvider' ? 'missing provider' : 'not imported';
    }
    return map;
  }, [model]);
  const modules = useMemo(() => {
    const roots = new Set(model.rootModules);
    return [...model.modules].sort((a, b) => (roots.has(b.name) ? 1 : 0) - (roots.has(a.name) ? 1 : 0) || a.name.localeCompare(b.name));
  }, [model]);
  const filesOf = (name: string) =>
    model.files
      .filter((f) => f.module === name)
      .sort((a, b) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind) || a.path.localeCompare(b.path));
  const loose = model.files.filter((f) => !f.module);
  const activeModule = moduleForFile(model, active)?.name;

  return (
    <div className="tree">
      {modules.map((m) => {
        const open = collapsed[m.name] === undefined ? m.name === activeModule : !collapsed[m.name];
        const issue = issuesByModule[m.name];
        const meta = issue ?? (model.rootModules.includes(m.name) ? 'root' : m.orm ?? (m.global ? 'global' : ''));
        return (
          <div key={m.name}>
            <button
              className="tree-row"
              onClick={() => setCollapsed((c) => ({ ...c, [m.name]: open }))}
              onDoubleClick={() => openFile(m.file, m.line)}
              onContextMenu={(e) => {
                e.preventDefault();
                setMenu({ x: e.clientX, y: e.clientY, items: moduleMenu(m) });
              }}
              aria-expanded={open}
            >
              <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} style={{ color: 'var(--text-2)' }} />
              <span className="kind module">M</span>
              <span className="label">{m.name}</span>
              {meta && (
                <span className="meta" style={issue ? { color: 'var(--warning)' } : undefined}>
                  {meta}
                </span>
              )}
            </button>
            {open &&
              filesOf(m.name).map((f) => (
                <button key={f.path} className={`tree-row ${f.path === active ? 'active' : ''} ${f.kind === 'spec' ? 'dim' : ''}`} style={{ paddingLeft: 40 }} onClick={() => openFile(f.path)} title={f.path}>
                  <span className={`kind ${f.kind}`}>{KIND_LETTER[f.kind]}</span>
                  <span className="label">{basename(f.path)}</span>
                  {model.diagnostics.some((d) => d.file === f.path && d.severity !== 'info') && <span className="dot warn" style={{ marginLeft: 'auto' }} aria-label="Has issues" />}
                </button>
              ))}
          </div>
        );
      })}
      {loose.length > 0 && (
        <>
          <div className="caps" style={{ padding: '12px 8px 4px', fontSize: 10.5 }}>
            Other files
          </div>
          {loose.map((f) => (
            <button key={f.path} className={`tree-row ${f.path === active ? 'active' : ''} ${f.kind === 'spec' ? 'dim' : ''}`} onClick={() => openFile(f.path)} title={f.path}>
              <span className={`kind ${f.kind}`}>{KIND_LETTER[f.kind]}</span>
              <span className="label">{f.path}</span>
            </button>
          ))}
        </>
      )}
      {menu && <ContextMenu {...menu} onClose={() => setMenu(null)} />}
    </div>
  );
}

function FileTree({ dir = '', depth = 0 }: { dir?: string; depth?: number }) {
  const [entries, setEntries] = useState<DirEntry[] | null>(null);
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const active = useApp((s) => s.active);
  const model = useApp((s) => s.model);
  useEffect(() => {
    api.listDir(dir).then(setEntries, () => setEntries([]));
  }, [dir, model]);
  if (!entries) return null;
  return (
    <>
      {entries.map((e) => (
        <div key={e.path}>
          <button
            className={`tree-row ${e.path === active ? 'active' : ''} ${e.ignored ? 'dim' : ''}`}
            style={{ paddingLeft: 8 + depth * 14 }}
            onClick={() => (e.dir ? setOpen((o) => ({ ...o, [e.path]: !o[e.path] })) : openFile(e.path))}
            title={e.path}
          >
            {e.dir ? <Icon name={open[e.path] ? 'chevronDown' : 'chevronRight'} size={12} /> : <span className={`kind ${kindOf(e.path)}`}>{KIND_LETTER[kindOf(e.path)]}</span>}
            <span className="label">{e.name}</span>
          </button>
          {e.dir && open[e.path] && <FileTree dir={e.path} depth={depth + 1} />}
        </div>
      ))}
    </>
  );
}

export function Explorer() {
  const model = useApp((s) => s.model)!;
  const active = useApp((s) => s.active);
  const [mode, setMode] = useState<'nest' | 'files'>('nest');
  const target = moduleForFile(model, active) ?? model.modules.find((m) => m.name === model.rootModules[0]);
  return (
    <aside className="side" aria-label="Project explorer">
      <div className="side-head">
        <span className="caps" style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>
          {model.name}
        </span>
        <div className="segmented">
          <button aria-pressed={mode === 'nest'} onClick={() => setMode('nest')}>
            Nest
          </button>
          <button aria-pressed={mode === 'files'} onClick={() => setMode('files')}>
            Files
          </button>
        </div>
      </div>
      <div className="scroll grow">{mode === 'nest' ? <NestTree /> : <div className="tree"><FileTree /></div>}</div>
      <div className="side-foot">
        <button className="btn" onClick={() => openGenerator(target ? dirname(target.file) : null, target?.name ?? null)}>
          <Icon name="plus" size={15} />
          Generate in {target?.name ?? 'project'}…
        </button>
        <div className="faint" style={{ fontSize: 11.5, textAlign: 'center' }}>
          Right-click any module for generators
        </div>
      </div>
    </aside>
  );
}

// ---- inspector -------------------------------------------------------------------------------
function InjectRow({ name, status, via }: { name: string; status: string; via: string | null }) {
  const icon = status === 'resolved' ? 'check' : status === 'missing' ? 'error' : 'info';
  const color = status === 'resolved' ? 'var(--success)' : status === 'missing' ? 'var(--error)' : 'var(--text-3)';
  return (
    <div className="row">
      <Icon name={icon} size={15} style={{ color }} />
      <span className="mono" style={{ fontSize: 12 }}>
        {name}
      </span>
      <span className="faint" style={{ marginLeft: 'auto', fontSize: 11.5 }}>
        {status === 'missing' ? 'not provided' : via ?? (status === 'unchecked' ? 'library' : '')}
      </span>
    </div>
  );
}

export function Inspector() {
  const model = useApp((s) => s.model)!;
  const active = useApp((s) => s.active);
  const cursor = useApp((s) => s.cursor);
  const classes = model.classes.filter((c) => c.file === active);
  const cls: ClassInfo | undefined = [...classes].reverse().find((c) => c.line <= cursor.line) ?? classes[0];
  if (!cls) {
    return (
      <aside className="side right" aria-label="Inspector">
        <div className="section">
          <span className="caps">Inspector</span>
          <span className="muted">Open a Nest file to see its routes, injections and module wiring.</span>
        </div>
        <ProjectSummary />
      </aside>
    );
  }
  const routes = model.routes.filter((r) => r.controller === cls.name);
  const mod = model.modules.find((m) => m.name === (cls.kind === 'module' ? cls.name : cls.modules[0]));
  const kindTone = cls.kind === 'module' ? 'module' : cls.kind === 'controller' ? 'controller' : cls.kind === 'entity' || cls.kind === 'schema' ? 'entity' : cls.kind === 'dto' ? 'dto' : 'service';
  return (
    <aside className="side right scroll" aria-label="Inspector">
      <div className="section">
        <span className="caps">Inspector</span>
        <div className="row">
          <span style={{ fontSize: 15, fontWeight: 600 }}>{cls.name}</span>
          <span className={`badge kind ${kindTone}`} style={{ width: 'auto', height: 'auto', font: 'inherit', fontSize: 11, padding: '2px 7px' }}>
            {cls.kind}
          </span>
        </div>
        {cls.prefix !== null && (
          <span className="mono muted" style={{ fontSize: 12 }}>
            prefix /{cls.prefix} · {cls.transport?.toUpperCase()}
          </span>
        )}
      </div>
      {routes.length > 0 && (
        <div className="section">
          <span className="muted" style={{ fontSize: 12, fontWeight: 600 }}>
            Routes
          </span>
          {routes.map((r) => (
            <button key={`${r.method}${r.path}${r.line}`} className="row" style={{ background: 'none', border: 0, padding: 0, cursor: 'pointer', textAlign: 'left' }} onClick={() => openFile(r.file, r.line)}>
              <span className={`method m-${r.method}`}>{r.method}</span>
              <span className="mono" style={{ fontSize: 12 }}>
                {r.path}
              </span>
            </button>
          ))}
        </div>
      )}
      {cls.injects.length > 0 && (
        <div className="section">
          <span className="muted" style={{ fontSize: 12, fontWeight: 600 }}>
            Injects
          </span>
          {cls.injects.map((i) => (
            <InjectRow key={i.param} name={i.token} status={i.status} via={i.via} />
          ))}
        </div>
      )}
      {(cls.kind === 'entity' || cls.kind === 'schema' || cls.kind === 'dto') && cls.fields.length > 0 && (
        <div className="section">
          <span className="muted" style={{ fontSize: 12, fontWeight: 600 }}>
            Fields
          </span>
          {cls.fields.map((f) => (
            <div key={f.name} className="row" style={{ fontSize: 12 }}>
              <span className="mono">
                {f.name}
                {f.optional ? '?' : ''}
              </span>
              <span className="mono faint" style={{ marginLeft: 'auto' }}>
                {f.typeText}
              </span>
            </div>
          ))}
          {cls.kind !== 'dto' && (
            <button className="btn sm" onClick={() => void guard(api.previewSyncDto(cls.name)).then((cs) => cs && showPreview(cs, { title: 'Sync DTO from entity' }))}>
              Sync Create DTO from columns
            </button>
          )}
        </div>
      )}
      {mod && (
        <div className="section" style={{ borderBottom: 0 }}>
          <span className="muted" style={{ fontSize: 12, fontWeight: 600 }}>
            {mod.name}
          </span>
          <div className="kv">
            <span>imports</span>
            <span>{mod.imports.map((i) => i.display).join(', ') || '—'}</span>
            <span>providers</span>
            <span>{mod.providers.map((p) => p.token).join(', ') || '—'}</span>
            <span>controllers</span>
            <span>{mod.controllers.join(', ') || '—'}</span>
            <span>exports</span>
            <span>{mod.exports.join(', ') || '—'}</span>
          </div>
          <button className="btn" style={{ marginTop: 6 }} onClick={() => setView('graph')}>
            Open in module graph
          </button>
        </div>
      )}
      {!mod && cls.kind !== 'module' && ['controller', 'service', 'provider'].includes(cls.kind) && (
        <div className="section">
          <span className="row" style={{ color: 'var(--warning)' }}>
            <Icon name="warning" size={15} /> Not registered in any module
          </span>
          <button className="btn sm" onClick={() => setView('graph')}>
            Fix in module graph
          </button>
        </div>
      )}
    </aside>
  );
}

function ProjectSummary() {
  const model = useApp((s) => s.model)!;
  return (
    <div className="section">
      <span className="muted" style={{ fontSize: 12, fontWeight: 600 }}>
        Project
      </span>
      <div className="kv">
        <span>modules</span>
        <span>{model.modules.length}</span>
        <span>providers</span>
        <span>{model.modules.reduce((n, m) => n + m.providers.length, 0)}</span>
        <span>routes</span>
        <span>{model.routes.length}</span>
        <span>scan</span>
        <span>{model.scanMs.toFixed(0)} ms</span>
      </div>
    </div>
  );
}

// ---- bottom panel ---------------------------------------------------------------------------
function LogView({ id }: { id: string }) {
  const lines = useApp((s) => s.logs[id]);
  const ref = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  useEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);
  return (
    <div
      className="log"
      ref={ref}
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 30;
      }}
    >
      {(lines ?? []).map((l, i) => (
        <div key={i} className={l.stream} dangerouslySetInnerHTML={{ __html: ansiToHtml(l.text) || '&nbsp;' }} />
      ))}
      {!lines?.length && <div className="faint">No output yet.</div>}
    </div>
  );
}

function ProblemsView() {
  const model = useApp((s) => s.model)!;
  const problems = useApp((s) => s.problems);
  const checking = useApp((s) => s.typechecking);
  const items = [
    ...model.diagnostics.map((d) => ({ key: d.id, sev: d.severity, text: `${d.title}: ${d.subject}`, detail: d.detail, file: d.file, line: d.line, source: 'graph' })),
    ...problems.map((p, i) => ({ key: `tsc${i}`, sev: p.severity, text: p.message, detail: p.code, file: p.file, line: p.line, source: 'tsc' })),
  ];
  return (
    <div className="scroll grow">
      <div className="row" style={{ padding: '6px 16px', borderBottom: '1px solid var(--border-subtle)' }}>
        <span className="faint" style={{ fontSize: 12 }}>
          Nest wiring checks run on every save. Type errors come from the project's own tsc.
        </span>
        <button className="btn sm" style={{ marginLeft: 'auto' }} disabled={checking} onClick={() => void typecheck()}>
          {checking ? 'Checking…' : 'Type check'}
        </button>
      </div>
      {items.length === 0 && <div className="empty" style={{ height: 'auto' }}>No problems found.</div>}
      {items.map((it) => (
        <div key={it.key} className="problem" onClick={() => it.file && openFile(it.file, it.line)} role="button" tabIndex={0}>
          <Icon name={it.sev === 'error' ? 'error' : it.sev === 'warning' ? 'warning' : 'info'} size={15} style={{ color: it.sev === 'error' ? 'var(--error)' : it.sev === 'warning' ? 'var(--warning)' : 'var(--info)', marginTop: 1 }} />
          <span className="col" style={{ gap: 2, minWidth: 0 }}>
            <span>{it.text}</span>
            <span className="faint" style={{ fontSize: 11.5 }}>
              {it.source === 'graph' ? it.detail : it.detail} {it.file && `· ${it.file}:${it.line}`}
            </span>
          </span>
        </div>
      ))}
    </div>
  );
}

function TestsSummary() {
  const run = useApp((s) => s.testRun);
  const busy = useApp((s) => s.testsRunning);
  return (
    <div className="scroll grow">
      <div className="row" style={{ padding: '6px 16px', borderBottom: '1px solid var(--border-subtle)' }}>
        {run ? (
          <span style={{ fontSize: 12 }}>
            <span style={{ color: 'var(--success)' }}>{run.passed} passed</span>
            {run.failed > 0 && <span style={{ color: 'var(--error)' }}> · {run.failed} failed</span>}
            {run.coverage && <span className="muted"> · coverage {run.coverage.linesPct}%</span>}
            <span className="faint"> · {(run.durationMs / 1000).toFixed(1)} s</span>
          </span>
        ) : (
          <span className="faint">Tests haven't run yet.</span>
        )}
        <button className="btn sm" style={{ marginLeft: 'auto' }} onClick={() => setView('tests')}>
          Open test explorer
        </button>
        <button className="btn sm primary" disabled={busy} onClick={() => void runTests()}>
          {busy ? 'Running…' : 'Run all'}
        </button>
      </div>
      <LogView id="tests" />
    </div>
  );
}

function TerminalView() {
  const [line, setLine] = useState('');
  const [history, setHistory] = useState<string[]>([]);
  const [hi, setHi] = useState(-1);
  const running = useApp((s) => s.running.terminal);
  return (
    <div className="col grow">
      <LogView id="tasks" />
      <form
        className="terminal-input"
        onSubmit={(e) => {
          e.preventDefault();
          if (!line.trim()) return;
          useApp.getState().logs.tasks;
          setHistory((h) => [line, ...h].slice(0, 50));
          setHi(-1);
          void guard(api.runCommand(line));
          setLine('');
        }}
      >
        <span className="mono faint">$</span>
        <input
          value={line}
          onChange={(e) => setLine(e.target.value)}
          placeholder={running ? 'A command is running — Enter runs another after stopping it' : 'Run a command in the project folder (npm, nest, git…)'}
          aria-label="Command"
          onKeyDown={(e) => {
            if (e.key === 'ArrowUp' && history.length) {
              const n = Math.min(hi + 1, history.length - 1);
              setHi(n);
              setLine(history[n]);
            }
            if (e.key === 'ArrowDown') {
              const n = Math.max(hi - 1, -1);
              setHi(n);
              setLine(n < 0 ? '' : history[n]);
            }
          }}
        />
        {running && (
          <button type="button" className="btn sm" onClick={() => void api.stopProcess('terminal')}>
            Stop
          </button>
        )}
        <button type="button" className="btn sm ghost" onClick={() => clearLog('tasks')}>
          Clear
        </button>
      </form>
    </div>
  );
}

export function BottomPanel() {
  const open = useApp((s) => s.panelOpen);
  const tab = useApp((s) => s.panelTab);
  const model = useApp((s) => s.model)!;
  const running = useApp((s) => s.running.run);
  const started = useApp((s) => s.appStarted);
  const script = useApp((s) => s.runScript);
  const problems = useApp((s) => s.problems.length + (s.model?.diagnostics.filter((d) => d.severity !== 'info').length ?? 0));
  const debugging = useApp((s) => !['idle', 'ended'].includes(s.debug.state));
  const [height, setHeight] = useState(() => {
    try {
      return Number(localStorage.getItem('nf.panelHeight')) || 216;
    } catch {
      return 216;
    }
  });
  if (!open) return null;
  const startDrag = (e: React.MouseEvent) => {
    const startY = e.clientY;
    const startH = height;
    const move = (ev: MouseEvent) => setHeight(Math.max(110, Math.min(window.innerHeight - 220, startH + startY - ev.clientY)));
    const up = () => {
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
      try {
        localStorage.setItem('nf.panelHeight', String(height));
      } catch {
        /* storage unavailable */
      }
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  };
  const tabs: [PanelTab, React.ReactNode][] = [
    [
      'run',
      <>
        <span className={`dot ${running || debugging ? (started || debugging ? 'ok' : 'warn') : 'off'}`} />
        {debugging ? 'Debug' : `Run · ${script}`}
      </>,
    ],
    ['problems', <>Problems {problems > 0 && <span className="count">{problems}</span>}</>],
    ['tests', 'Tests'],
    ['terminal', 'Terminal'],
  ];
  return (
    <>
      <div className="resizer" onMouseDown={startDrag} role="separator" aria-orientation="horizontal" aria-label="Resize panel" />
      <section className="panel" style={{ height }} aria-label="Panel">
        <div role="tablist" className="panel-tabs">
          {tabs.map(([id, label]) => (
            <button key={id} role="tab" aria-selected={tab === id} onClick={() => useApp.setState({ panelTab: id })}>
              {label}
            </button>
          ))}
          <span className="faint" style={{ marginLeft: 'auto', fontSize: 11.5 }}>
            {tab === 'run' && running && started && `→ http://localhost:${model.port}${model.globalPrefix ? `/${model.globalPrefix}` : ''}`}
          </span>
          {tab === 'run' && running && started && (
            <button className="btn sm ghost" onClick={() => void openUrl(`http://localhost:${model.port}/docs`)}>
              Open Swagger
            </button>
          )}
          {tab === 'run' && (
            <button className="btn sm ghost" onClick={() => clearLog(debugging ? 'debug' : 'run')}>
              Clear
            </button>
          )}
          <button className="icon-btn" aria-label="Hide panel" onClick={() => useApp.setState({ panelOpen: false })}>
            <Icon name="chevronDown" size={14} />
          </button>
        </div>
        {tab === 'run' && <LogView id={debugging ? 'debug' : 'run'} />}
        {tab === 'problems' && <ProblemsView />}
        {tab === 'tests' && <TestsSummary />}
        {tab === 'terminal' && <TerminalView />}
      </section>
    </>
  );
}

export function Workspace() {
  return (
    <>
      <Explorer />
      <main className="main">
        <EditorArea />
        <BottomPanel />
      </main>
      <Inspector />
    </>
  );
}

export function copyText(text: string) {
  void navigator.clipboard?.writeText(stripAnsi(text)).then(() => toast('Copied'));
}
