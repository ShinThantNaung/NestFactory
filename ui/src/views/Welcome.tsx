import { useEffect, useRef, useState } from 'react';
import { api, on, pickFolder } from '../api';
import { Icon, Logo } from '../icons';
import { useApp, openProject, setView, toast, errorText, clearLog } from '../store';
import { openProjectDialog } from '../components/Shell';
import { ansiToHtml, basename } from '../util';
import type { WizardOptions } from '../bindings/WizardOptions';
import type { WizardPlan } from '../bindings/WizardPlan';
import type { PackageManager } from '../bindings/PackageManager';

export function Welcome() {
  const settings = useApp((s) => s.settings);
  const tc = useApp((s) => s.toolchain);
  const busy = useApp((s) => s.busy);
  return (
    <div className="welcome">
      <div className="welcome-card">
        <div className="col" style={{ gap: 28 }}>
          <div>
            <Logo size={44} />
            <h1 style={{ marginTop: 18 }}>NestFactory</h1>
            <p className="muted" style={{ fontSize: 14, lineHeight: '22px', margin: 0, maxWidth: 420 }}>
              The fastest path from an empty folder to a running, documented, database-backed NestJS API. Generators, the module graph, database setup and tests in one window.
            </p>
          </div>
          <div className="col" style={{ gap: 10 }}>
            <button className="action-tile" onClick={() => useApp.setState({ screen: 'wizard' })}>
              <span className="icon-tile">
                <Icon name="plus" size={16} />
              </span>
              <span className="col" style={{ gap: 2 }}>
                <span style={{ fontWeight: 600 }}>New project</span>
                <span className="muted" style={{ fontSize: 12 }}>
                  Wraps nest new, with config, auth, Swagger and health starters
                </span>
              </span>
            </button>
            <button className="action-tile" onClick={() => void openProjectDialog()} disabled={!!busy}>
              <span className="icon-tile" style={{ background: 'var(--c-module-bg)', color: 'var(--c-module)' }}>
                <Icon name="folder" size={16} />
              </span>
              <span className="col" style={{ gap: 2 }}>
                <span style={{ fontWeight: 600 }}>{busy ?? 'Open project'}</span>
                <span className="muted" style={{ fontSize: 12 }}>
                  Any folder with a Nest package.json · Ctrl O
                </span>
              </span>
            </button>
          </div>
        </div>
        <div className="col" style={{ gap: 22 }}>
          <section className="col" style={{ gap: 8 }}>
            <span className="caps">Recent</span>
            <div className="recent">
              {!settings?.recentProjects.length && <span className="faint">No recent projects.</span>}
              {settings?.recentProjects.map((p) => (
                <button key={p} onClick={() => void openProject(p)}>
                  <span style={{ fontWeight: 500 }}>{basename(p.replace(/\\/g, '/'))}</span>
                  <span className="faint mono" style={{ fontSize: 11.5 }}>
                    {p}
                  </span>
                </button>
              ))}
            </div>
          </section>
          <section className="col" style={{ gap: 8 }}>
            <span className="caps">Toolchain</span>
            {tc ? (
              <div className="col" style={{ gap: 6, fontSize: 12.5 }}>
                {[
                  ['Node.js', tc.node],
                  ['Nest CLI', tc.nest ?? (tc.node ? 'via npx' : null)],
                  ['Docker', tc.docker?.replace('Docker version ', '') ?? null],
                  ['Git', tc.git?.replace('git version ', '') ?? null],
                ].map(([k, v]) => (
                  <div key={k} className="row">
                    <span className={`dot ${v ? 'ok' : 'off'}`} />
                    <span style={{ width: 80 }}>{k}</span>
                    <span className="mono muted">{v ?? 'not found'}</span>
                  </div>
                ))}
              </div>
            ) : (
              <span className="faint">Checking…</span>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}

type StepStatus = { status: string; detail: string };

export function Wizard() {
  const settings = useApp((s) => s.settings);
  const tc = useApp((s) => s.toolchain);
  const [opts, setOpts] = useState<WizardOptions>(() => ({
    name: 'my-api',
    location: settings?.defaultLocation ?? '',
    monorepo: false,
    packageManager: (settings?.defaultPackageManager ?? 'npm') as PackageManager,
    strict: true,
    lint: true,
    config: true,
    auth: false,
    swagger: true,
    health: true,
    database: true,
  }));
  const [plan, setPlan] = useState<WizardPlan | null>(null);
  const [running, setRunning] = useState(false);
  const [steps, setSteps] = useState<Record<number, StepStatus>>({});
  const [failed, setFailed] = useState<string | null>(null);
  const logs = useApp((s) => s.logs.wizard);
  const logRef = useRef<HTMLDivElement>(null);
  const sections = useRef<(HTMLElement | null)[]>([]);
  const [current, setCurrent] = useState(0);

  useEffect(() => {
    const t = setTimeout(() => {
      api.wizardPlan(opts).then(setPlan, () => {});
    }, 150);
    return () => clearTimeout(t);
  }, [opts]);

  useEffect(() => on('wizard-step', (p: { index: number; status: string; detail: string }) => setSteps((s) => ({ ...s, [p.index]: { status: p.status, detail: p.detail } }))), []);
  useEffect(() => {
    if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [logs]);

  const set = (patch: Partial<WizardOptions>) => setOpts((o) => ({ ...o, ...patch }));
  const available = (pm: PackageManager) => !tc || tc.packageManagers.includes(pm);

  const create = async () => {
    setRunning(true);
    setFailed(null);
    setSteps({});
    clearLog('wizard');
    try {
      const path = await api.wizardRun(opts);
      toast(`Created ${opts.name}`, 'success');
      await openProject(path);
      if (opts.database) setView('database');
    } catch (e) {
      setFailed(errorText(e));
    } finally {
      setRunning(false);
    }
  };

  const SECTIONS = ['Project & layout', 'Tooling', 'Starter modules', 'Database (optional)'];
  const scrollTo = (i: number) => {
    setCurrent(i);
    sections.current[i]?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };

  return (
    <div className="body">
      <aside className="side" style={{ maxWidth: 280, flexBasis: 240, padding: '28px 20px', gap: 28 }}>
        <div className="col" style={{ gap: 6 }}>
          <span style={{ fontSize: 22, fontWeight: 600, letterSpacing: '-0.01em' }}>Start a Nest project</span>
          <span className="muted" style={{ lineHeight: '19px' }}>
            Wraps <span className="mono" style={{ color: 'var(--text)' }}>nest new</span>. Everything it writes builds and runs without NestFactory.
          </span>
        </div>
        <ol style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 4 }}>
          {SECTIONS.map((s, i) => (
            <li key={s}>
              <button className={`wizard-step ${i === current ? 'current' : ''}`} style={{ width: '100%', background: i === current ? undefined : 'none', border: 0, cursor: 'pointer' }} onClick={() => scrollTo(i)}>
                <span className="num">{i + 1}</span>
                {s}
              </button>
            </li>
          ))}
        </ol>
        <div className="col" style={{ marginTop: 'auto', gap: 8 }}>
          <span className="caps">Team templates</span>
          <span className="faint" style={{ fontSize: 12, lineHeight: '18px' }}>
            Generators pick up team templates from <span className="mono">.nestfactory/templates/&lt;schematic&gt;</span> in each repo, using the Nest schematic template syntax.
          </span>
        </div>
      </aside>

      <main className="main" style={{ overflow: 'auto' }}>
        {running || failed ? (
          <div className="page-inner" style={{ maxWidth: 820 }}>
            <h1>{failed ? 'Project creation stopped' : `Creating ${opts.name}…`}</h1>
            <ol className="steps">
              {(plan?.steps ?? []).map((s, i) => {
                const st = steps[i];
                const icon = !st ? 'chevronRight' : st.status === 'done' ? 'check' : st.status === 'failed' ? 'error' : 'restart';
                const color = !st ? 'var(--text-3)' : st.status === 'done' ? 'var(--success)' : st.status === 'failed' ? 'var(--error)' : 'var(--warning)';
                return (
                  <li key={i}>
                    <Icon name={icon} size={16} style={{ color, marginTop: 1 }} />
                    <span className="col" style={{ gap: 3 }}>
                      <span>{s.title}</span>
                      {s.command && <span className="mono muted" style={{ fontSize: 11.5 }}>{s.command.join(' ')}</span>}
                    </span>
                  </li>
                );
              })}
            </ol>
            <div ref={logRef} className="log cmd" style={{ maxHeight: 340, minHeight: 200 }}>
              {(logs ?? []).map((l, i) => (
                <div key={i} dangerouslySetInnerHTML={{ __html: ansiToHtml(l.text) || '&nbsp;' }} />
              ))}
            </div>
            {failed && (
              <>
                <div className="note" style={{ borderColor: 'var(--error-bg)' }}>
                  <Icon name="error" size={16} style={{ color: 'var(--error)' }} />
                  <span className="selectable" style={{ whiteSpace: 'pre-wrap' }}>{failed}</span>
                </div>
                <div className="row">
                  <button className="btn lg" onClick={() => setFailed(null)}>
                    Back to options
                  </button>
                </div>
              </>
            )}
          </div>
        ) : (
          <>
            <div className="page-inner" style={{ maxWidth: 760, gap: 28, padding: '32px 40px' }}>
              <section ref={(el) => { sections.current[0] = el; }} className="col" style={{ gap: 20 }}>
                <div className="form-grid" style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(min(240px, 100%), 1fr))' }}>
                  <label className="field">
                    <span>Project name</span>
                    <input className="input" value={opts.name} onChange={(e) => set({ name: e.target.value.toLowerCase().replace(/[^a-z0-9-_]/g, '-') })} autoFocus />
                  </label>
                  <label className="field">
                    <span>Location</span>
                    <span className="row">
                      <input className="input mono grow" value={opts.location} onChange={(e) => set({ location: e.target.value })} />
                      <button
                        className="btn"
                        style={{ height: 36 }}
                        onClick={async () => {
                          const p = await pickFolder('Create the project in');
                          if (p) set({ location: p });
                        }}
                      >
                        Browse
                      </button>
                    </span>
                  </label>
                </div>
                <fieldset>
                  <legend className="legend">Layout</legend>
                  <div className="form-grid" style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(min(240px, 100%), 1fr))' }}>
                    {[
                      [false, 'Standard', 'One app in src/. Best for a single API.'],
                      [true, 'Monorepo', 'apps/ and libs/, shared code across services.'],
                    ].map(([mono, title, desc]) => (
                      <label key={String(mono)} className={`option ${opts.monorepo === mono ? 'selected' : ''}`} style={{ padding: 14, alignItems: 'flex-start', gap: 12 }}>
                        <input type="radio" name="layout" checked={opts.monorepo === mono} onChange={() => set({ monorepo: mono as boolean })} style={{ marginTop: 2 }} />
                        <span className="col" style={{ gap: 4 }}>
                          <span style={{ fontWeight: 600 }}>{title as string}</span>
                          <span className="muted" style={{ lineHeight: '18px' }}>
                            {desc as string}
                          </span>
                        </span>
                      </label>
                    ))}
                  </div>
                </fieldset>
              </section>

              <section ref={(el) => { sections.current[1] = el; }} className="row" style={{ flexWrap: 'wrap', gap: 28, alignItems: 'flex-start' }}>
                <fieldset>
                  <legend className="legend">Package manager</legend>
                  <div className="segmented" style={{ padding: 3, background: 'var(--bg-panel)', border: '1px solid var(--border-strong)', borderRadius: 8 }}>
                    {(['npm', 'pnpm', 'yarn'] as PackageManager[]).map((pm) => (
                      <button key={pm} aria-pressed={opts.packageManager === pm} disabled={!available(pm)} title={available(pm) ? '' : `${pm} isn't installed`} style={{ height: 30, padding: '0 16px', fontSize: 12.5 }} onClick={() => set({ packageManager: pm })}>
                        {pm}
                      </button>
                    ))}
                  </div>
                </fieldset>
                <fieldset className="col" style={{ gap: 10 }}>
                  <legend className="legend">Tooling</legend>
                  <label className="check">
                    <input type="checkbox" checked={opts.strict} onChange={(e) => set({ strict: e.target.checked })} />
                    Strict TypeScript
                  </label>
                  <label className="check">
                    <input type="checkbox" checked={opts.lint} onChange={(e) => set({ lint: e.target.checked })} />
                    ESLint + Prettier
                  </label>
                </fieldset>
              </section>

              <fieldset ref={(el) => { sections.current[2] = el; }}>
                <legend className="legend">Starter modules</legend>
                <div className="grid-options" style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(min(160px, 100%), 1fr))', gap: 10 }}>
                  {(
                    [
                      ['config', 'Config', '@nestjs/config'],
                      ['auth', 'Auth', 'JWT + Passport'],
                      ['swagger', 'Swagger', '@nestjs/swagger'],
                      ['health', 'Health check', '@nestjs/terminus'],
                    ] as const
                  ).map(([key, title, pkg]) => {
                    const checked = opts[key] || (key === 'config' && opts.auth);
                    return (
                      <label key={key} className={`option ${checked ? 'selected' : ''}`} style={{ flexDirection: 'column', alignItems: 'flex-start', gap: 8, padding: 12 }}>
                        <span className="row" style={{ fontWeight: 600 }}>
                          <input type="checkbox" checked={checked} disabled={key === 'config' && opts.auth} onChange={(e) => set({ [key]: e.target.checked } as Partial<WizardOptions>)} />
                          {title}
                        </span>
                        <span className="mono muted" style={{ fontSize: 11.5 }}>
                          {pkg}
                        </span>
                      </label>
                    );
                  })}
                </div>
              </fieldset>

              <div ref={(el) => { sections.current[3] = el; }} className="card">
                <Icon name="database" style={{ color: 'var(--info)' }} />
                <span className="col" style={{ gap: 2, flex: '1 1 260px' }}>
                  <span style={{ fontWeight: 600 }}>Connect a database now</span>
                  <span className="muted">TypeORM · PostgreSQL in Docker — opens the connection manager after create</span>
                </span>
                <label className="check">
                  <input type="checkbox" checked={opts.database} onChange={(e) => set({ database: e.target.checked })} />
                  Include
                </label>
              </div>
            </div>
            <div className="dialog-foot" style={{ padding: '14px 40px', position: 'sticky', bottom: 0 }}>
              <span className="faint" style={{ fontSize: 12 }}>
                {plan?.error ?? 'About 40 s with a warm package cache'}
              </span>
              <button className="btn lg" style={{ marginLeft: 'auto' }} onClick={() => useApp.setState({ screen: useApp.getState().model ? 'project' : 'welcome' })}>
                Cancel
              </button>
              <button className="btn lg primary" disabled={!!plan?.error || !opts.name} onClick={() => void create()}>
                Create project
              </button>
            </div>
          </>
        )}
      </main>

      <aside className="side right" aria-label="Preview" style={{ flexBasis: 340, maxWidth: 420, padding: '28px 22px', gap: 22, overflow: 'auto' }}>
        <div className="col" style={{ gap: 10 }}>
          <span className="caps">Will run</span>
          <div className="cmd mono" style={{ lineHeight: '20px' }}>
            {(plan?.commands ?? []).map((c, i) => (
              <div key={i}>
                <span className="faint">$</span> {c}
              </div>
            ))}
            <div className="faint"># then writes the starter modules</div>
          </div>
        </div>
        <div className="col" style={{ gap: 10 }}>
          <span className="caps">Files</span>
          <div className="mono col" style={{ gap: 3, fontSize: 12, lineHeight: '19px' }}>
            <span className="muted">{opts.name || 'project'}/</span>
            {(plan?.files ?? []).map((f) => (
              <span key={f.path} style={{ paddingLeft: f.path.includes('/') ? 28 : 14 }}>
                {f.added && <span style={{ color: 'var(--success)' }}>+ </span>}
                {f.path.includes('/') ? f.path.split('/').slice(f.path.startsWith('apps/') ? 3 : 1).join('/') || f.path : f.path}
                {f.note && <span className="faint"> · {f.note}</span>}
              </span>
            ))}
          </div>
        </div>
        <div className="note ok" style={{ marginTop: 'auto', color: 'var(--text)' }}>
          <Icon name="check" size={16} style={{ color: 'var(--success)', marginTop: 1 }} />
          <span>No NestFactory files are added. The project matches Nest CLI output, so terminal and VS Code users can work on it too.</span>
        </div>
      </aside>
    </div>
  );
}
