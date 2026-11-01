import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../api';
import { Icon } from '../icons';
import { useApp, applyChangeset, errorText, openFile } from '../store';
import { basename, dirname } from '../util';
import { DiffView } from './Editor';
import type { Changeset } from '../bindings/Changeset';
import type { Schematic } from '../bindings/Schematic';
import type { Transport } from '../bindings/Transport';
import type { DtoField } from '../bindings/DtoField';
import type { GenerateOptions } from '../bindings/GenerateOptions';

const STATUS_LETTER = { added: 'A', modified: 'M', deleted: 'D' } as const;

export function ChangesView({ cs, emptyText }: { cs: Changeset | null; emptyText?: string }) {
  const [selected, setSelected] = useState(0);
  const [inline, setInline] = useState(true);
  useEffect(() => setSelected(0), [cs?.id]);
  if (!cs) {
    return (
      <section aria-label="Changes preview" className="grow" style={{ display: 'flex' }}>
        <div className="empty">{emptyText ?? 'Fill in the form to preview the changes.'}</div>
      </section>
    );
  }
  const file = cs.files[Math.min(selected, cs.files.length - 1)];
  const [firstNote, ...otherNotes] = cs.notes;
  const installs = [...cs.install, ...cs.installDev.map((p) => `${p} (dev)`)];
  return (
    <section aria-label="Changes preview" style={{ flex: '999 1 560px', minWidth: 0, display: 'flex', minHeight: 0 }}>
      <div className="change-list">
        <span className="caps" style={{ padding: '0 8px 10px' }}>
          {cs.files.length} change{cs.files.length === 1 ? '' : 's'}
        </span>
        {cs.files.map((f, i) => (
          <button key={f.path} aria-current={i === selected} onClick={() => setSelected(i)} title={f.path} style={{ color: f.path.includes('.spec.') ? 'var(--text-2)' : undefined }}>
            <span className={`st st-${f.status}`}>{STATUS_LETTER[f.status]}</span>
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{shortPath(f.path, cs)}</span>
            {f.status === 'modified' && (
              <span style={{ marginLeft: 'auto', color: 'var(--warning)', fontSize: 11.5 }}>
                +{f.additions}
                {f.deletions ? ` −${f.deletions}` : ''}
              </span>
            )}
          </button>
        ))}
        {firstNote && (
          <div className="note" style={{ marginTop: 14, fontSize: 12 }}>
            <span>{firstNote}</span>
          </div>
        )}
        {installs.length > 0 && (
          <div className="note" style={{ marginTop: 8, fontSize: 12 }}>
            <span>
              Installs <span className="mono" style={{ color: 'var(--text)' }}>{installs.join(' ')}</span> after writing.
            </span>
          </div>
        )}
      </div>
      <div style={{ flex: '999 1 420px', minWidth: 0, display: 'flex', flexDirection: 'column' }}>
        {file ? (
          <>
            <div className="row" style={{ height: 44, flex: 'none', padding: '0 18px', borderBottom: '1px solid var(--border-subtle)' }}>
              <span className="mono" style={{ fontSize: 12.5 }}>
                {file.path}
              </span>
              <span className={`badge ${file.status === 'deleted' ? 'err' : 'ok'}`}>
                {file.status === 'added' ? `new · ${file.additions} lines` : file.status === 'deleted' ? 'deleted' : `${file.additions} addition${file.additions === 1 ? '' : 's'}${file.deletions ? `, ${file.deletions} removal${file.deletions === 1 ? '' : 's'}` : ''}`}
              </span>
              <span className="segmented" style={{ marginLeft: 'auto' }}>
                <button aria-pressed={inline} onClick={() => setInline(true)}>
                  Inline
                </button>
                <button aria-pressed={!inline} onClick={() => setInline(false)}>
                  Split
                </button>
              </span>
            </div>
            <div style={{ flex: 1, position: 'relative', minHeight: 200 }}>
              <DiffView path={file.path} original={file.before ?? ''} modified={file.after ?? ''} inline={inline} />
            </div>
          </>
        ) : (
          <div className="empty">Nothing to change.</div>
        )}
        {otherNotes.map((n) => (
          <div key={n} className="note" style={{ margin: '0 18px 12px', flex: 'none' }}>
            <Icon name="check" size={16} style={{ color: 'var(--success)', marginTop: 1 }} />
            <span>{n}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

function shortPath(path: string, cs: Changeset) {
  const added = cs.files.filter((f) => f.status === 'added').map((f) => dirname(f.path));
  const base = added.length ? added.reduce((a, b) => (b.length < a.length ? b : a)) : '';
  if (base && path.startsWith(`${base}/`)) return path.slice(base.length + 1);
  return basename(path);
}

export function PreviewDialog() {
  const preview = useApp((s) => s.preview);
  const [writing, setWriting] = useState(false);
  if (!preview) return null;
  const cs = preview.changeset;
  const close = () => {
    void api.discardChangeset(cs.id);
    useApp.setState({ preview: null });
  };
  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="dialog" role="dialog" aria-label={preview.title ?? cs.label}>
        <div className="dialog-head">
          <span className="icon-tile">
            <Icon name="check" size={16} />
          </span>
          <h2>{preview.title ?? cs.label}</h2>
          {preview.title && <span className="faint">{cs.label}</span>}
          <button className="icon-btn" style={{ marginLeft: 'auto' }} aria-label="Close dialog" onClick={close}>
            <Icon name="close" size={16} />
          </button>
        </div>
        <div className="split-v">
          <ChangesView cs={cs} />
        </div>
        <div className="dialog-foot">
          <span className="muted" style={{ fontSize: 12.5 }}>
            Writes as one step — <span className="mono" style={{ color: 'var(--text)' }}>Ctrl Z</span> undoes all {cs.files.length} changes
          </span>
          <button className="btn lg" style={{ marginLeft: 'auto' }} onClick={close}>
            Cancel
          </button>
          <button
            className="btn lg primary"
            disabled={writing || cs.files.length === 0}
            onClick={async () => {
              setWriting(true);
              await applyChangeset(cs, preview.onApplied);
              setWriting(false);
            }}
          >
            Write {cs.files.length} change{cs.files.length === 1 ? '' : 's'}
          </button>
        </div>
      </div>
    </div>
  );
}

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

export function GeneratorDialog() {
  const gen = useApp((s) => s.generator);
  const model = useApp((s) => s.model);
  const [schematic, setSchematic] = useState<Schematic>(gen?.schematic ?? 'resource');
  const [name, setName] = useState('');
  const [directory, setDirectory] = useState<string | null>(gen?.directory ?? null);
  const [moduleName, setModuleName] = useState<string | null>(gen?.module ?? null);
  const [transport, setTransport] = useState<Transport>('rest');
  const [crud, setCrud] = useState(true);
  const [swagger, setSwagger] = useState(true);
  const [spec, setSpec] = useState(true);
  const [flat, setFlat] = useState(false);
  const [fields, setFields] = useState<DtoField[]>([{ name: '', ty: 'string', optional: false, array: false }]);
  const [cs, setCs] = useState<Changeset | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [writing, setWriting] = useState(false);
  const lastId = useRef<string | null>(null);
  const nameRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!gen) return;
    setSchematic(gen.schematic);
    setDirectory(gen.directory);
    setModuleName(gen.module);
    setName('');
    setCs(null);
    setError(null);
    setTimeout(() => nameRef.current?.focus(), 0);
  }, [gen]);

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
    if (!gen) return;
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
  }, [opts, gen, name]);

  if (!gen || !model) return null;
  const close = () => {
    if (lastId.current) void api.discardChangeset(lastId.current);
    lastId.current = null;
    useApp.setState({ generator: null });
  };
  const write = async () => {
    if (!cs) return;
    setWriting(true);
    lastId.current = null;
    const ok = await applyChangeset(cs);
    setWriting(false);
    if (ok) {
      const main = cs.files.find((f) => f.status === 'added' && /\.(controller|service|module|guard|pipe|interceptor|filter|gateway|resolver|dto|middleware)\.ts$/.test(f.path) && !f.path.includes('.spec.'));
      if (main) openFile(main.path);
    }
  };
  const modules = [...model.modules].sort((a, b) => a.name.localeCompare(b.name));
  const showTransport = schematic === 'resource';
  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="dialog" role="dialog" aria-label="Generate">
        <div className="dialog-head">
          <span className="icon-tile">
            <Icon name="plus" size={16} />
          </span>
          <h2>Generate</h2>
          <span className="faint">in</span>
          <select
            className="badge warn"
            style={{ border: 0, fontSize: 12, fontWeight: 500, padding: '3px 8px', borderRadius: 6, cursor: 'pointer' }}
            value={moduleName ?? ''}
            aria-label="Target module"
            onChange={(e) => {
              const m = model.modules.find((x) => x.name === e.target.value);
              setModuleName(m?.name ?? null);
              setDirectory(m ? dirname(m.file) : model.sourceRoot);
            }}
          >
            {!moduleName && <option value="">{model.sourceRoot}/</option>}
            {modules.map((m) => (
              <option key={m.name} value={m.name}>
                {m.name}
              </option>
            ))}
          </select>
          <button className="icon-btn" style={{ marginLeft: 'auto' }} aria-label="Close dialog" onClick={close}>
            <Icon name="close" size={16} />
          </button>
        </div>
        <div className="split-v">
          <form
            style={{ flex: '1 1 380px', maxWidth: 440, padding: 22, borderRight: '1px solid var(--border-subtle)', display: 'flex', flexDirection: 'column', gap: 22, overflow: 'auto' }}
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
            {showTransport && (
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
                <button type="button" className="btn sm dashed" onClick={() => setFields([...fields, { name: '', ty: 'string', optional: false, array: false }])}>
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
              {schematic !== 'dto' && schematic !== 'module' && schematic !== 'decorator' && (
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
            <div className="col" style={{ marginTop: 'auto', gap: 6 }}>
              <span className="caps">Same as</span>
              <span className="cmd mono">{cs?.command ?? `nest g ${schematic === 'dto' ? 'class' : schematic} ${name || '<name>'}`}</span>
            </div>
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
        <div className="dialog-foot">
          <span className="muted" style={{ fontSize: 12.5 }}>
            Writes as one step — <span className="mono" style={{ color: 'var(--text)' }}>Ctrl Z</span> undoes all {cs?.files.length ?? 0} changes
          </span>
          <button className="btn lg" style={{ marginLeft: 'auto' }} onClick={close}>
            Cancel
          </button>
          <button className="btn lg primary" disabled={!cs || writing} onClick={() => void write()}>
            Write {cs?.files.length ?? 0} change{cs?.files.length === 1 ? '' : 's'}
          </button>
        </div>
      </div>
    </div>
  );
}
