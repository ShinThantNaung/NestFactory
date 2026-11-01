import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../api';
import { Icon } from '../icons';
import { useApp, applyChangeset, errorText, openFile } from '../store';
import { ChangesView } from '../components/Changes';
import { dirname } from '../util';
import type { Changeset } from '../bindings/Changeset';
import type { Schematic } from '../bindings/Schematic';
import type { Transport } from '../bindings/Transport';
import type { DtoField } from '../bindings/DtoField';
import type { GenerateOptions } from '../bindings/GenerateOptions';

const KINDS: [Schematic, string][] = [
  ['resource', 'Resource'],
  ['controller', 'Controller'],
  ['service', 'Service'],
  ['guard', 'Guard'],
  ['pipe', 'Pipe'],
  ['interceptor', 'Interceptor'],
  ['filter', 'Filter'],
  ['dto', 'DTO'],
  ['gateway', 'Gateway'],
  ['module', 'Module'],
  ['middleware', 'Middleware'],
  ['resolver', 'Resolver'],
  ['provider', 'Provider'],
];

const TRANSPORTS: [Transport, string, string?][] = [
  ['rest', 'REST API'],
  ['graphqlCodeFirst', 'GraphQL', 'code first'],
  ['graphqlSchemaFirst', 'GraphQL', 'schema first'],
  ['microservice', 'Microservice', 'non-HTTP'],
  ['ws', 'WebSockets'],
];

const BLANK_FIELD: DtoField = { name: '', ty: 'string', optional: false, array: false };

function classify(name: string) {
  return name
    .split(/[-_\s/.]+/)
    .filter(Boolean)
    .map((p) => p.charAt(0).toUpperCase() + p.slice(1))
    .join('');
}

function helperText(schematic: Schematic, raw: string, module: string | null) {
  const base = raw.split('/').pop() ?? '';
  const n = classify(base.replace(/([a-z])([A-Z])/g, '$1-$2'));
  if (!n) return 'Use kebab-case or camelCase; folders with "/".';
  const where = module ? ` in ${module}` : '';
  switch (schematic) {
    case 'resource':
      return `Creates ${n}Module, ${n}Controller, ${n}Service`;
    case 'module':
      return `Creates ${n}Module, imported by the nearest module`;
    case 'dto':
      return `Creates ${n.replace(/Dto$/, '')}Dto`;
    case 'provider':
      return `Creates ${n}${where}`;
    default: {
      const suffix = schematic.charAt(0).toUpperCase() + schematic.slice(1);
      const wired = ['controller', 'service', 'gateway', 'resolver'].includes(schematic) ? where : '';
      return `Creates ${n}${suffix}${wired}`;
    }
  }
}

/** The generator screen: form on the left, every file change previewed on the right. */
export function GeneratorView() {
  const gen = useApp((s) => s.generator);
  const model = useApp((s) => s.model)!;
  const [schematic, setSchematic] = useState<Schematic>(gen?.schematic ?? 'resource');
  const [name, setName] = useState('');
  const [directory, setDirectory] = useState<string | null>(gen?.directory ?? null);
  const [moduleName, setModuleName] = useState<string | null>(gen?.module ?? null);
  const [transport, setTransport] = useState<Transport>('rest');
  const [crud, setCrud] = useState(true);
  const hasSwagger = model.dependencies.includes('@nestjs/swagger');
  const [swagger, setSwagger] = useState(hasSwagger);
  const [spec, setSpec] = useState(true);
  const [flat, setFlat] = useState(false);
  const [fields, setFields] = useState<DtoField[]>([BLANK_FIELD]);
  const [cs, setCs] = useState<Changeset | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [writing, setWriting] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const lastId = useRef<string | null>(null);
  const nameRef = useRef<HTMLInputElement>(null);

  // A new target (from the explorer, the graph or the rail) resets the form.
  useEffect(() => {
    if (!gen) return;
    setSchematic(gen.schematic);
    setDirectory(gen.directory);
    setModuleName(gen.module);
    setTimeout(() => nameRef.current?.focus(), 0);
  }, [gen]);

  useEffect(
    () => () => {
      if (lastId.current) void api.discardChangeset(lastId.current);
    },
    [],
  );

  const opts: GenerateOptions = useMemo(
    () => ({
      schematic,
      name,
      directory,
      transport,
      crud,
      spec: schematic === 'dto' ? false : spec,
      flat,
      swagger,
      skipImport: false,
      fields: schematic === 'dto' ? fields.filter((f) => f.name.trim()) : [],
    }),
    [schematic, name, directory, transport, crud, spec, flat, swagger, fields],
  );

  useEffect(() => {
    if (!name.trim()) {
      setCs(null);
      setError(null);
      return;
    }
    const t = setTimeout(async () => {
      try {
        const next = await api.previewGenerate(opts);
        if (lastId.current) void api.discardChangeset(lastId.current);
        lastId.current = next.id;
        setCs(next);
        setError(null);
      } catch (e) {
        setCs(null);
        setError(errorText(e));
      }
    }, 180);
    return () => clearTimeout(t);
  }, [opts, name]);

  const reset = () => {
    setName('');
    setTransport('rest');
    setCrud(true);
    setSwagger(hasSwagger);
    setSpec(true);
    setFlat(false);
    setFields([BLANK_FIELD]);
    nameRef.current?.focus();
  };

  const write = async () => {
    if (!cs) return;
    setWriting(true);
    lastId.current = null;
    const ok = await applyChangeset(cs);
    setWriting(false);
    if (ok) {
      reset();
      const added = cs.files.filter((f) => f.status === 'added' && !f.path.includes('.spec.'));
      const order = ['controller', 'resolver', 'gateway', 'service', 'module', 'guard', 'pipe', 'interceptor', 'filter', 'middleware', 'dto'];
      const main = order.map((k) => added.find((f) => f.path.endsWith(`.${k}.ts`))).find(Boolean) ?? added[0];
      if (main) openFile(main.path);
    }
  };

  const modules = [...model.modules].sort((a, b) => a.name.localeCompare(b.name));
  const target = moduleName ?? `${model.sourceRoot}/`;
  const command = cs?.command ?? `nest g ${schematic === 'dto' ? 'class' : schematic} ${name || '<name>'}`;
  const n = cs?.files.length ?? 0;

  return (
    <main className="main">
      <div className="row" style={{ minHeight: 48, flex: 'none', padding: '8px 16px', borderBottom: '1px solid var(--border-subtle)', background: 'var(--bg-panel)', flexWrap: 'wrap', gap: 10 }}>
        <span style={{ fontSize: 14, fontWeight: 600 }}>Generate</span>
        <span className="faint">in</span>
        <div style={{ position: 'relative' }}>
          <button className="row" aria-haspopup="listbox" aria-expanded={menuOpen} onClick={() => setMenuOpen((o) => !o)} style={{ height: 28, gap: 6, padding: '0 10px', background: 'var(--c-module-bg)', border: 0, borderRadius: 6, color: 'var(--c-module)', fontSize: 12, fontWeight: 500, cursor: 'pointer' }}>
            {target}
            <Icon name="chevronDown" size={13} />
          </button>
          {menuOpen && (
            <>
              <div style={{ position: 'fixed', inset: 0, zIndex: 49 }} onMouseDown={() => setMenuOpen(false)} />
              <div className="menu" role="listbox" style={{ left: 0, top: 32, maxHeight: 360, overflow: 'auto' }}>
                {modules.map((m) => (
                  <button
                    key={m.name}
                    role="option"
                    aria-selected={m.name === moduleName}
                    onClick={() => {
                      setModuleName(m.name);
                      setDirectory(dirname(m.file));
                      setMenuOpen(false);
                    }}
                  >
                    <span className="kind module">M</span>
                    {m.name}
                    <span className="kbd mono">{dirname(m.file)}</span>
                  </button>
                ))}
              </div>
            </>
          )}
        </div>
        <span className="row muted" style={{ marginLeft: 'auto', gap: 8, fontSize: 12, minWidth: 0 }}>
          Same as
          <span className="mono selectable" style={{ padding: '4px 8px', background: 'var(--bg-base)', border: '1px solid var(--border-subtle)', borderRadius: 6, color: 'var(--text)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {command}
          </span>
        </span>
      </div>

      <div className="split-v">
        <form
          className="gen-form"
          onSubmit={(e) => {
            e.preventDefault();
            void write();
          }}
        >
          <fieldset>
            <legend className="legend">What to generate</legend>
            <div className="row" style={{ flexWrap: 'wrap', gap: 6 }}>
              {KINDS.map(([k, label]) => (
                <button key={k} type="button" className="chip" aria-pressed={schematic === k} onClick={() => setSchematic(k)}>
                  {label}
                </button>
              ))}
            </div>
          </fieldset>
          <label className="field">
            <span>Name</span>
            <input ref={nameRef} className="input mono" value={name} placeholder={schematic === 'dto' ? 'create-invoice' : schematic === 'resource' ? 'invoices' : 'name'} onChange={(e) => setName(e.target.value)} spellCheck={false} />
            <span className="hint">{helperText(schematic, name, moduleName)}</span>
          </label>
          {schematic === 'resource' && (
            <fieldset className="col" style={{ gap: 8 }}>
              <legend className="legend">Transport layer</legend>
              {TRANSPORTS.map(([t, label, sub]) => (
                <label key={t} className={`option ${transport === t ? 'selected' : ''}`}>
                  <input type="radio" name="transport" checked={transport === t} onChange={() => setTransport(t)} />
                  {label} {sub && <span className="faint">· {sub}</span>}
                </label>
              ))}
            </fieldset>
          )}
          {schematic === 'dto' && (
            <fieldset className="col" style={{ gap: 8 }}>
              <legend className="legend">Fields</legend>
              {fields.map((f, i) => (
                <div key={i} className="row">
                  <input className="input sm mono grow" placeholder="field" value={f.name} aria-label="Field name" onChange={(e) => setFields(fields.map((x, j) => (j === i ? { ...x, name: e.target.value } : x)))} />
                  <select className="input sm" value={f.ty} aria-label="Field type" onChange={(e) => setFields(fields.map((x, j) => (j === i ? { ...x, ty: e.target.value } : x)))}>
                    {['string', 'number', 'int', 'boolean', 'date', 'email', 'uuid', 'url'].map((t) => (
                      <option key={t}>{t}</option>
                    ))}
                  </select>
                  <label className="check" title="Optional">
                    <input type="checkbox" checked={f.optional} onChange={(e) => setFields(fields.map((x, j) => (j === i ? { ...x, optional: e.target.checked } : x)))} />?
                  </label>
                  <button type="button" className="icon-btn" aria-label="Remove field" onClick={() => setFields(fields.filter((_, j) => j !== i))}>
                    <Icon name="close" size={13} />
                  </button>
                </div>
              ))}
              <button type="button" className="btn sm dashed" onClick={() => setFields([...fields, BLANK_FIELD])}>
                <Icon name="plus" size={13} /> Add field
              </button>
            </fieldset>
          )}
          <fieldset className="col" style={{ gap: 10 }}>
            <legend className="legend">Options</legend>
            {schematic === 'resource' && (
              <label className="check">
                <input type="checkbox" checked={crud} onChange={(e) => setCrud(e.target.checked)} />
                Generate CRUD entry points
              </label>
            )}
            {(schematic === 'resource' || schematic === 'dto') && (
              <label className="check">
                <input type="checkbox" checked={swagger} onChange={(e) => setSwagger(e.target.checked)} />
                Swagger decorators on DTOs
              </label>
            )}
            {schematic !== 'dto' && schematic !== 'module' && (
              <label className="check">
                <input type="checkbox" checked={spec} onChange={(e) => setSpec(e.target.checked)} />
                Spec files
              </label>
            )}
            <label className="check">
              <input type="checkbox" checked={flat} onChange={(e) => setFlat(e.target.checked)} />
              Flat (no folder)
            </label>
          </fieldset>
        </form>
        {error ? (
          <section className="grow" style={{ display: 'flex' }}>
            <div className="empty">
              <Icon name="warning" size={24} style={{ color: 'var(--warning)' }} />
              <span>{error}</span>
            </div>
          </section>
        ) : (
          <ChangesView cs={cs} emptyText="Type a name to preview every file the generator writes." />
        )}
      </div>

      <div className="dialog-foot" style={{ padding: '12px 16px' }}>
        <span className="muted" style={{ fontSize: 12.5 }}>
          Writes as one step — <span className="mono" style={{ color: 'var(--text)' }}>Ctrl Z</span> undoes all {n} changes
        </span>
        <button className="btn lg" style={{ marginLeft: 'auto' }} onClick={reset}>
          Reset
        </button>
        <button className="btn lg primary" disabled={!cs || writing} onClick={() => void write()}>
          Write {n} change{n === 1 ? '' : 's'}
        </button>
      </div>
    </main>
  );
}
