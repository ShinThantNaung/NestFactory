import { useEffect, useMemo, useState } from 'react';
import { api, on } from '../api';
import { Icon } from '../icons';
import { useApp, guard, toast, errorText, showPreview, setView } from '../store';
import { ChangesView } from '../components/Changes';
import { dirname } from '../util';
import type { DbConfig } from '../bindings/DbConfig';
import type { DbProfile } from '../bindings/DbProfile';
import type { DbTestResult } from '../bindings/DbTestResult';
import type { DockerStatus } from '../bindings/DockerStatus';
import type { Orm } from '../bindings/Orm';
import type { Database } from '../bindings/Database';
import type { DbPlan } from '../bindings/DbPlan';
import type { EntityField } from '../bindings/EntityField';

const ORMS: [Orm, string][] = [
  ['typeOrm', 'TypeORM'],
  ['prisma', 'Prisma'],
  ['mongoose', 'Mongoose'],
  ['mikroOrm', 'MikroORM'],
  ['sequelize', 'Sequelize'],
];

const DBS: [Database, string, number][] = [
  ['postgres', 'PostgreSQL', 5432],
  ['mysql', 'MySQL', 3306],
  ['sqlite', 'SQLite', 0],
  ['mongodb', 'MongoDB', 27017],
];

const ORM_LABEL = Object.fromEntries(ORMS);
const DB_LABEL = Object.fromEntries(DBS.map(([k, l]) => [k, l]));

function supports(orm: Orm, db: Database) {
  if (orm === 'mongoose') return db === 'mongodb';
  if (orm === 'sequelize') return db !== 'mongodb';
  return true;
}

function randomPassword() {
  const bytes = new Uint8Array(12);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => 'abcdefghijkmnpqrstuvwxyz23456789'[b % 32]).join('');
}

function projectSlug(name: string) {
  return name.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/_api$/, '').replace(/^_+|_+$/g, '') || 'app';
}

export function detectOrm(deps: string[]): Orm {
  if (deps.includes('@prisma/client') || deps.includes('prisma')) return 'prisma';
  if (deps.includes('@nestjs/mongoose')) return 'mongoose';
  if (deps.includes('@mikro-orm/nestjs')) return 'mikroOrm';
  if (deps.includes('@nestjs/sequelize')) return 'sequelize';
  return 'typeOrm';
}

function defaultConfig(projectName: string, deps: string[]): DbConfig {
  const slug = projectSlug(projectName);
  const orm = detectOrm(deps);
  const database: Database = orm === 'mongoose' ? 'mongodb' : 'postgres';
  return {
    name: `${slug}_dev`,
    orm,
    database,
    host: 'localhost',
    port: database === 'mongodb' ? 27017 : 5432,
    databaseName: `${slug}_dev`,
    user: database === 'mongodb' ? 'root' : 'postgres',
    password: randomPassword(),
    synchronize: true,
    docker: true,
  };
}

function containerName(project: string) {
  const slug = project.toLowerCase().replace(/[^a-z0-9]/g, '-').replace(/-api$/, '').replace(/^-+|-+$/g, '');
  return `${slug}-db`;
}

function serviceName(db: Database) {
  return db === 'postgres' ? 'postgres' : db === 'mysql' ? 'mysql' : 'mongo';
}

const STEP_ORDER = ['packages', 'files', 'env', 'docker', 'test'] as const;
const STEP_LABEL: Record<string, string> = {
  packages: 'Installed packages',
  files: 'Wrote module config',
  env: 'Added variables to .env',
  docker: 'Started Docker container',
  test: 'Tested the connection',
};

type StepState = { status: string; detail: string };

export function DatabaseView() {
  const model = useApp((s) => s.model)!;
  const [profiles, setProfiles] = useState<DbProfile[]>([]);
  const [selected, setSelected] = useState<string | 'new'>('new');
  const [cfg, setCfg] = useState<DbConfig>(() => defaultConfig(model.name, model.dependencies));
  const [result, setResult] = useState<DbTestResult | null>(null);
  const [testing, setTesting] = useState(false);
  const [steps, setSteps] = useState<Record<string, StepState>>({});
  const [plan, setPlan] = useState<DbPlan | null>(null);
  const [applying, setApplying] = useState(false);
  const [docker, setDocker] = useState<DockerStatus | null>(null);

  const profile = profiles.find((p) => p.config.name === selected);
  const container = profile?.container ?? containerName(model.name);
  const service = profile?.service ?? serviceName(cfg.database);

  useEffect(() => {
    api.dbProfiles().then((ps) => {
      setProfiles(ps);
      if (ps.length) void selectProfile(ps[0]);
    }, () => {});
    return on('db-step', (p: { step: string; status: string; detail: string }) => setSteps((s) => ({ ...s, [p.step]: { status: p.status, detail: p.detail } })));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!cfg.docker || cfg.database === 'sqlite') {
      setDocker(null);
      return;
    }
    let alive = true;
    const poll = () => api.dockerStatus(container).then((d) => alive && setDocker(d), () => {});
    void poll();
    const t = setInterval(poll, 5000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [cfg.docker, cfg.database, container]);

  async function selectProfile(p: DbProfile) {
    setSelected(p.config.name);
    const password = (await api.dbPassword(p.config.name).catch(() => null)) ?? '';
    setCfg({ ...p.config, password });
    setSteps({});
    setResult(null);
    setTesting(true);
    const r = await api.dbTest({ ...p.config, password }).catch((e) => ({ ok: false, error: errorText(e), latencyMs: 0, version: null, tcpOnly: false }));
    setResult(r);
    setTesting(false);
  }

  const update = (patch: Partial<DbConfig>) => setCfg((c) => ({ ...c, ...patch }));
  const setDb = (database: Database) => {
    const port = DBS.find((d) => d[0] === database)![2];
    const user = database === 'mysql' ? 'root' : database === 'mongodb' ? 'root' : database === 'postgres' ? 'postgres' : '';
    const orm = supports(cfg.orm, database) ? cfg.orm : 'typeOrm';
    const databaseName = database === 'sqlite' ? 'dev.sqlite' : cfg.databaseName.endsWith('.sqlite') ? `${projectSlug(model.name)}_dev` : cfg.databaseName;
    update({ database, port, user, orm, databaseName, docker: database !== 'sqlite' && cfg.docker });
  };
  const setOrm = (orm: Orm) => {
    if (orm === 'mongoose') setDb('mongodb');
    update({ orm, ...(orm === 'mongoose' ? {} : {}) });
    if (!supports(orm, cfg.database)) setDb(orm === 'mongoose' ? 'mongodb' : 'postgres');
  };

  const test = async () => {
    setTesting(true);
    try {
      setResult(await api.dbTest(cfg));
    } catch (e) {
      setResult({ ok: false, error: errorText(e), latencyMs: 0, version: null, tcpOnly: false });
    } finally {
      setTesting(false);
    }
  };

  const previewSetup = async () => {
    try {
      setPlan(await api.dbPlan(cfg));
    } catch (e) {
      toast(errorText(e), 'error');
    }
  };

  const runSetup = async () => {
    if (!plan) return;
    const id = plan.changeset.id;
    setPlan(null);
    setApplying(true);
    setSteps({});
    useApp.setState({ panelOpen: true });
    try {
      const r = await api.dbApply(cfg, id);
      setResult(r);
      const ps = await api.dbProfiles();
      setProfiles(ps);
      setSelected(cfg.name);
      if (r.ok) toast(`Connected to ${DB_LABEL[cfg.database]}${r.version ? ` · ${r.version}` : ''}`, 'success');
      else toast(r.error ?? 'The connection test failed', 'error');
    } catch (e) {
      toast(errorText(e), 'error');
    } finally {
      setApplying(false);
    }
  };

  const envPreview = useMemo(() => {
    if (cfg.orm === 'prisma') return [['DATABASE_URL', '••••••••']];
    if (cfg.database === 'mongodb') return [['MONGODB_URI', '••••••••']];
    if (cfg.database === 'sqlite') return [['DB_NAME', cfg.databaseName]];
    return [
      ['DB_HOST', cfg.host],
      ['DB_PORT', String(cfg.port)],
      ['DB_NAME', cfg.databaseName],
      ['DB_USER', cfg.user],
      ['DB_PASSWORD', '••••••••'],
    ];
  }, [cfg]);

  const usedBy = profile?.moduleFile ? profile.moduleFile.split('/').pop()!.replace('.module.ts', '').replace(/(^|-)(\w)/g, (_m, _d, c: string) => c.toUpperCase()) + 'Module' : null;
  const statusPill = testing ? (
    <span className="pill" style={{ background: 'var(--bg-raised)', color: 'var(--text-2)' }}>
      Testing…
    </span>
  ) : result ? (
    result.ok ? (
      <span className="pill" style={{ background: 'var(--success-bg)', color: 'var(--success)' }}>
        <span className="dot ok" />
        {result.tcpOnly ? 'Port open · driver not installed yet' : `Connected · ${Math.round(result.latencyMs)} ms`}
      </span>
    ) : (
      <span className="pill" style={{ background: 'var(--error-bg)', color: 'var(--error)' }} title={result.error ?? ''}>
        <span className="dot err" />
        Not connected
      </span>
    )
  ) : (
    <span className="pill" style={{ background: 'var(--bg-raised)', color: 'var(--text-2)' }}>
      <span className="dot off" />
      Not set up
    </span>
  );

  return (
    <>
      <aside className="side" aria-label="Connections" style={{ maxWidth: 250, flexBasis: 220, padding: '14px 10px', gap: 4 }}>
        <span className="caps" style={{ padding: '0 8px 10px' }}>
          Connections
        </span>
        {profiles.map((p) => (
          <button
            key={p.config.name}
            className="tree-row"
            style={{ height: 'auto', flexDirection: 'column', alignItems: 'flex-start', gap: 3, padding: 10, background: selected === p.config.name ? 'var(--bg-selected)' : undefined }}
            onClick={() => void selectProfile(p)}
          >
            <span className="row" style={{ fontWeight: 500 }}>
              <span className={`dot ${selected === p.config.name && result?.ok ? 'ok' : 'off'}`} />
              {p.config.name}
            </span>
            <span className="muted" style={{ paddingLeft: 15, fontSize: 11.5 }}>
              {ORM_LABEL[p.config.orm]} · {DB_LABEL[p.config.database]}
              {p.config.docker ? ' · Docker' : ''}
            </span>
          </button>
        ))}
        <button
          className="btn dashed"
          style={{ marginTop: 8 }}
          onClick={() => {
            setSelected('new');
            setCfg(defaultConfig(model.name, model.dependencies));
            setResult(null);
            setSteps({});
          }}
        >
          <Icon name="plus" size={14} /> New connection
        </button>
      </aside>

      <main className="page">
        <div className="page-inner">
          <div className="row" style={{ alignItems: 'flex-end', flexWrap: 'wrap', gap: 12 }}>
            <div className="col" style={{ gap: 4 }}>
              <h1>{selected === 'new' ? 'New connection' : cfg.name}</h1>
              <span className="muted">
                {usedBy ? (
                  <>
                    Used by <span className="mono" style={{ color: 'var(--text)' }}>{usedBy}</span> in development
                  </>
                ) : (
                  'NestFactory installs the packages, writes the module config and .env, and tests the connection.'
                )}
              </span>
            </div>
            <span style={{ marginLeft: 'auto' }}>{statusPill}</span>
          </div>

          <label className="field" style={{ maxWidth: 320 }}>
            <span>Connection name</span>
            <input className="input mono" value={cfg.name} onChange={(e) => update({ name: e.target.value.replace(/[^\w-]/g, '_') })} />
          </label>

          <fieldset>
            <legend className="legend">ORM / ODM</legend>
            <div className="grid-options">
              {ORMS.map(([k, label]) => (
                <label key={k} className={`option ${cfg.orm === k ? 'selected' : ''}`}>
                  <input type="radio" name="orm" checked={cfg.orm === k} onChange={() => setOrm(k)} />
                  {label}
                </label>
              ))}
            </div>
          </fieldset>

          <fieldset>
            <legend className="legend">Database</legend>
            <div className="grid-options">
              {DBS.map(([k, label]) => {
                const ok = supports(cfg.orm, k);
                return (
                  <label key={k} className={`option ${cfg.database === k ? 'selected' : ''} ${ok ? '' : 'disabled'}`}>
                    <input type="radio" name="db" checked={cfg.database === k} disabled={!ok} onChange={() => setDb(k)} />
                    {label}
                    {!ok && <span style={{ marginLeft: 'auto', fontSize: 10.5 }}>n/a</span>}
                  </label>
                );
              })}
            </div>
          </fieldset>

          {cfg.database !== 'sqlite' && (
            <div className="card">
              <Icon name="docker" style={{ color: 'var(--info)' }} />
              <span className="col" style={{ gap: 2, flex: '1 1 240px' }}>
                <span style={{ fontWeight: 600 }}>Local container</span>
                <span className="mono muted" style={{ fontSize: 12 }}>
                  {cfg.database === 'postgres' ? 'postgres:16' : cfg.database === 'mysql' ? 'mysql:8.4' : 'mongo:7'} · {container}
                </span>
              </span>
              <label className="check" style={{ fontSize: 12.5 }}>
                <input type="checkbox" checked={cfg.docker} onChange={(e) => update({ docker: e.target.checked })} />
                Run in Docker
              </label>
              {cfg.docker && docker && (
                <>
                  <span className="row" style={{ fontSize: 12, color: docker.running ? 'var(--success)' : 'var(--text-2)' }}>
                    <span className={`dot ${docker.running ? 'ok' : 'off'}`} />
                    {!docker.available ? 'Docker not running' : docker.running ? 'Running' : docker.exists ? 'Stopped' : 'Not created'}
                  </span>
                  {docker.available && docker.exists && (
                    <button
                      className="btn sm"
                      onClick={() =>
                        void guard(docker.running ? api.dockerStop(service) : api.dockerStart(service)).then(() => api.dockerStatus(container).then(setDocker))
                      }
                    >
                      {docker.running ? 'Stop' : 'Start'}
                    </button>
                  )}
                </>
              )}
            </div>
          )}

          <section className="form-grid">
            {cfg.database !== 'sqlite' && (
              <>
                <label className="field">
                  <span>Host</span>
                  <input className="input mono" value={cfg.host} onChange={(e) => update({ host: e.target.value })} />
                </label>
                <label className="field">
                  <span>Port</span>
                  <input className="input mono" inputMode="numeric" value={cfg.port} onChange={(e) => update({ port: Number(e.target.value.replace(/\D/g, '')) || 0 })} />
                </label>
              </>
            )}
            <label className="field">
              <span>{cfg.database === 'sqlite' ? 'Database file' : 'Database'}</span>
              <input className="input mono" value={cfg.databaseName} onChange={(e) => update({ databaseName: e.target.value })} />
            </label>
            {cfg.database !== 'sqlite' && (
              <>
                <label className="field">
                  <span>User</span>
                  <input className="input mono" value={cfg.user} onChange={(e) => update({ user: e.target.value })} />
                </label>
                <label className="field">
                  <span>Password</span>
                  <input className="input mono" type="password" value={cfg.password} onChange={(e) => update({ password: e.target.value })} />
                  <span className="row hint">
                    <Icon name="lock" size={13} />
                    Saved in the OS keychain, not in project files
                  </span>
                </label>
              </>
            )}
            {(cfg.orm === 'typeOrm' || cfg.orm === 'sequelize') && (
              <label className="check" style={{ alignSelf: 'start', marginTop: 28 }}>
                <input type="checkbox" checked={cfg.synchronize} onChange={(e) => update({ synchronize: e.target.checked })} />
                Synchronize schema in dev
              </label>
            )}
          </section>

          <div className="row" style={{ flexWrap: 'wrap' }}>
            <button className="btn lg primary" disabled={applying} onClick={() => void previewSetup()}>
              {applying ? 'Setting up…' : profile ? 'Re-apply setup…' : 'Preview & set up'}
            </button>
            <button className="btn lg" disabled={testing} onClick={() => void test()}>
              Test connection
            </button>
            {selected !== 'new' && (
              <button
                className="btn lg ghost"
                onClick={async () => {
                  if (!window.confirm(`Remove the ${cfg.name} profile? Project files stay as they are.`)) return;
                  const ps = await guard(api.dbDeleteProfile(cfg.name));
                  if (ps) {
                    setProfiles(ps);
                    setSelected('new');
                    setCfg(defaultConfig(model.name, model.dependencies));
                    setResult(null);
                  }
                }}
              >
                Remove profile
              </button>
            )}
          </div>
          {result?.error && !result.ok && (
            <div className="note" style={{ borderColor: 'var(--error-bg)' }}>
              <Icon name="error" size={16} style={{ color: 'var(--error)' }} />
              <span className="selectable">{result.error}</span>
            </div>
          )}
        </div>
      </main>

      <aside className="side right" aria-label="Setup" style={{ flexBasis: 320, maxWidth: 380, padding: '24px 20px', gap: 18 }}>
        <span className="caps">{Object.keys(steps).length ? 'What NestFactory did' : 'What NestFactory will do'}</span>
        <ol className="steps">
          {STEP_ORDER.filter((s) => s !== 'docker' || (cfg.docker && cfg.database !== 'sqlite')).map((s) => {
            const st = steps[s];
            const icon = !st ? 'chevronRight' : st.status === 'done' ? 'check' : st.status === 'failed' ? 'error' : 'restart';
            const color = !st ? 'var(--text-3)' : st.status === 'done' ? 'var(--success)' : st.status === 'failed' ? 'var(--error)' : 'var(--warning)';
            return (
              <li key={s}>
                <Icon name={icon} size={16} style={{ color, marginTop: 1 }} />
                <span className="col" style={{ gap: 3, minWidth: 0 }}>
                  <span>{STEP_LABEL[s]}</span>
                  <span className="mono muted" style={{ fontSize: 11.5, whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>
                    {st?.detail || plannedDetail(s, cfg)}
                  </span>
                </span>
              </li>
            );
          })}
        </ol>
        <div className="cmd mono" style={{ lineHeight: '20px' }}>
          <span className="faint"># .env</span>
          {envPreview.map(([k, v]) => (
            <div key={k}>
              <span style={{ color: 'var(--info)' }}>{k}</span>={v.includes('•') ? <span className="faint">{v}</span> : v}
            </div>
          ))}
        </div>
        <div className="row" style={{ marginTop: 'auto', flexWrap: 'wrap' }}>
          <button className="btn grow" disabled={testing} onClick={() => void test()}>
            Test again
          </button>
          <button className="btn grow primary" onClick={() => useApp.setState({ entityDesigner: {} })}>
            Generate entity…
          </button>
        </div>
      </aside>

      {plan && (
        <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && setPlan(null)}>
          <div className="dialog" role="dialog" aria-label="Database setup preview">
            <div className="dialog-head">
              <span className="icon-tile">
                <Icon name="database" size={16} />
              </span>
              <h2>{plan.changeset.label}</h2>
              <button className="icon-btn" style={{ marginLeft: 'auto' }} aria-label="Close dialog" onClick={() => setPlan(null)}>
                <Icon name="close" size={16} />
              </button>
            </div>
            <div className="split-v">
              <ChangesView cs={plan.changeset} />
            </div>
            <div className="dialog-foot">
              <span className="muted" style={{ fontSize: 12.5 }}>
                Then installs packages{plan.docker ? `, starts ${plan.docker.container}` : ''} and tests the connection. File changes undo with Ctrl Z.
              </span>
              <button className="btn lg" style={{ marginLeft: 'auto' }} onClick={() => setPlan(null)}>
                Cancel
              </button>
              <button className="btn lg primary" onClick={() => void runSetup()}>
                Write {plan.changeset.files.length} changes and connect
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

function plannedDetail(step: string, cfg: DbConfig) {
  switch (step) {
    case 'packages':
      return {
        typeOrm: '@nestjs/typeorm typeorm',
        prisma: 'prisma @prisma/client',
        mongoose: '@nestjs/mongoose mongoose',
        mikroOrm: '@mikro-orm/core @mikro-orm/nestjs',
        sequelize: '@nestjs/sequelize sequelize sequelize-typescript',
      }[cfg.orm];
    case 'files':
      return cfg.orm === 'prisma' ? 'prisma/schema.prisma, PrismaModule' : 'src/database/database.module.ts';
    case 'env':
      return 'gitignored; validated by the Config schema';
    case 'docker':
      return `${cfg.database === 'postgres' ? 'postgres:16' : cfg.database === 'mysql' ? 'mysql:8.4' : 'mongo:7'} · port ${cfg.port}`;
    case 'test':
      return `${DB_LABEL[cfg.database]} with the project's own driver`;
    default:
      return '';
  }
}

// ---- entity designer --------------------------------------------------------------------------
const TYPES = ['string', 'text', 'int', 'float', 'decimal', 'boolean', 'date', 'uuid', 'email', 'json'];

export function EntityDesigner() {
  const state = useApp((s) => s.entityDesigner);
  const model = useApp((s) => s.model);
  const [name, setName] = useState('');
  const [module, setModule] = useState<string>('');
  const [orm, setOrm] = useState<Orm>('typeOrm');
  const [fields, setFields] = useState<EntityField[]>([
    { name: 'name', ty: 'string', nullable: false, unique: false, length: 120 },
  ]);
  const [dtos, setDtos] = useState(true);
  const [swagger, setSwagger] = useState(true);
  const [timestamps, setTimestamps] = useState(true);

  useEffect(() => {
    if (!state || !model) return;
    const m = state.module ?? model.modules.find((x) => !model.rootModules.includes(x.name) && !x.global && x.name !== 'DatabaseModule')?.name ?? model.rootModules[0] ?? '';
    setModule(m);
    setOrm(detectOrm(model.dependencies));
    setName('');
  }, [state, model]);

  if (!state || !model) return null;
  const close = () => useApp.setState({ entityDesigner: null });
  const mod = model.modules.find((m) => m.name === module);
  const directory = state.directory ?? (mod ? dirname(mod.file) : model.sourceRoot);
  const update = (i: number, patch: Partial<EntityField>) => setFields(fields.map((f, j) => (j === i ? { ...f, ...patch } : f)));
  const preview = async () => {
    try {
      const cs = await api.previewEntity({ name, directory, module: module || null, orm, fields, dtos, swagger, timestamps });
      close();
      showPreview(cs, { title: `Design entity ${name}` });
    } catch (e) {
      toast(errorText(e), 'error');
    }
  };
  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="dialog" role="dialog" aria-label="Entity designer" style={{ maxWidth: 980 }}>
        <div className="dialog-head">
          <span className="icon-tile" style={{ background: 'var(--c-entity-bg)', color: 'var(--c-entity)' }}>
            <Icon name="table" size={16} />
          </span>
          <h2>Entity designer</h2>
          <span className="faint">entity, DTOs with validation, Swagger decorators and forFeature registration</span>
          <button className="icon-btn" style={{ marginLeft: 'auto' }} aria-label="Close dialog" onClick={close}>
            <Icon name="close" size={16} />
          </button>
        </div>
        <div className="scroll grow" style={{ padding: 22, display: 'flex', flexDirection: 'column', gap: 20 }}>
          <div className="form-grid">
            <label className="field">
              <span>Entity name</span>
              <input className="input mono" placeholder="invoice" value={name} onChange={(e) => setName(e.target.value)} autoFocus />
            </label>
            <label className="field">
              <span>Module</span>
              <select className="input" value={module} onChange={(e) => setModule(e.target.value)}>
                {model.modules.map((m) => (
                  <option key={m.name} value={m.name}>
                    {m.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span>ORM</span>
              <select className="input" value={orm} onChange={(e) => setOrm(e.target.value as Orm)}>
                {ORMS.map(([k, l]) => (
                  <option key={k} value={k}>
                    {l}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <table className="table">
            <thead>
              <tr>
                <th>Column</th>
                <th style={{ width: 140 }}>Type</th>
                <th style={{ width: 90 }}>Length</th>
                <th style={{ width: 80 }}>Nullable</th>
                <th style={{ width: 70 }}>Unique</th>
                <th style={{ width: 40 }} />
              </tr>
            </thead>
            <tbody>
              <tr>
                <td className="mono faint">id</td>
                <td className="faint">primary key</td>
                <td />
                <td />
                <td />
                <td />
              </tr>
              {fields.map((f, i) => (
                <tr key={i}>
                  <td>
                    <input className="input sm mono" value={f.name} aria-label="Column name" onChange={(e) => update(i, { name: e.target.value })} />
                  </td>
                  <td>
                    <select className="input sm" value={f.ty} aria-label="Column type" onChange={(e) => update(i, { ty: e.target.value })}>
                      {TYPES.map((t) => (
                        <option key={t}>{t}</option>
                      ))}
                    </select>
                  </td>
                  <td>
                    <input
                      className="input sm mono"
                      disabled={!['string', 'email'].includes(f.ty)}
                      value={f.length ?? ''}
                      aria-label="Length"
                      onChange={(e) => update(i, { length: e.target.value ? Number(e.target.value.replace(/\D/g, '')) : null })}
                    />
                  </td>
                  <td>
                    <input type="checkbox" checked={f.nullable} aria-label="Nullable" onChange={(e) => update(i, { nullable: e.target.checked })} />
                  </td>
                  <td>
                    <input type="checkbox" checked={f.unique} aria-label="Unique" onChange={(e) => update(i, { unique: e.target.checked })} />
                  </td>
                  <td>
                    <button className="icon-btn" aria-label="Remove column" onClick={() => setFields(fields.filter((_, j) => j !== i))}>
                      <Icon name="trash" size={14} />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <button className="btn dashed" style={{ alignSelf: 'flex-start' }} onClick={() => setFields([...fields, { name: '', ty: 'string', nullable: false, unique: false, length: null }])}>
            <Icon name="plus" size={14} /> Add column
          </button>
          <div className="row" style={{ gap: 20, flexWrap: 'wrap' }}>
            <label className="check">
              <input type="checkbox" checked={dtos} onChange={(e) => setDtos(e.target.checked)} />
              Create and update DTOs with class-validator rules
            </label>
            <label className="check">
              <input type="checkbox" checked={swagger} onChange={(e) => setSwagger(e.target.checked)} />
              Swagger @ApiProperty
            </label>
            <label className="check">
              <input type="checkbox" checked={timestamps} onChange={(e) => setTimestamps(e.target.checked)} />
              createdAt / updatedAt
            </label>
          </div>
        </div>
        <div className="dialog-foot">
          <span className="muted" style={{ fontSize: 12.5 }}>
            Writes to <span className="mono">{directory}/</span>
          </span>
          <button className="btn lg" style={{ marginLeft: 'auto' }} onClick={close}>
            Cancel
          </button>
          <button className="btn lg primary" disabled={!name.trim()} onClick={() => void preview()}>
            Preview changes
          </button>
        </div>
      </div>
    </div>
  );
}

export function goDatabase() {
  setView('database');
}
