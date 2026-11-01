import { useEffect, useMemo, useState } from 'react';
import { api } from '../api';
import { Icon } from '../icons';
import { useApp, guard, toast, openFile } from '../store';
import type { EnvReport } from '../bindings/EnvReport';
import type { EnvIssue } from '../bindings/EnvIssue';

const SECRET = /(PASSWORD|SECRET|TOKEN|KEY|PRIVATE|_URL|_URI)$/i;

type Row = { key: string; value: string; original?: string; removed?: boolean };

export function EnvView() {
  const model = useApp((s) => s.model)!;
  const [report, setReport] = useState<EnvReport | null>(null);
  const [file, setFile] = useState('.env');
  const [rows, setRows] = useState<Row[]>([]);
  const [reveal, setReveal] = useState<Record<string, boolean>>({});

  const load = (r: EnvReport, f = file) => {
    setReport(r);
    const ef = r.files.find((x) => x.path === f) ?? r.files[0];
    if (ef && ef.path !== f) setFile(ef.path);
    setRows((ef?.entries ?? []).map((e) => ({ key: e.key, value: e.value, original: e.value })));
  };

  useEffect(() => {
    api.envReport().then((r) => load(r), () => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [model]);

  const issues = useMemo(() => (report?.issues ?? []).filter((i) => i.file === file), [report, file]);
  const dirty = rows.some((r) => r.removed || r.value !== r.original || r.original === undefined);
  const schemaFor = (k: string) => report?.schema.find((s) => s.key === k);

  const save = async () => {
    const updates: [string, string | null][] = [];
    for (const r of rows) {
      if (!r.key.trim()) continue;
      if (r.removed) updates.push([r.key, null]);
      else if (r.value !== r.original || r.original === undefined) updates.push([r.key.trim(), r.value]);
    }
    const next = await guard(api.envSave(file, updates));
    if (next) {
      load(next, file);
      toast(`Saved ${file}`, 'success');
    }
  };

  const fixIssue = (i: EnvIssue) => {
    if (i.kind === 'missing' && !rows.some((r) => r.key === i.key)) {
      const s = schemaFor(i.key);
      setRows([...rows, { key: i.key, value: s?.default?.replace(/^['"]|['"]$/g, '') ?? '' }]);
    } else if (i.kind === 'unused') {
      setRows(rows.map((r) => (r.key === i.key ? { ...r, removed: true } : r)));
    }
  };

  return (
    <>
      <aside className="side" aria-label="Environment files" style={{ maxWidth: 250 }}>
        <div className="side-head">
          <span className="caps">Env files</span>
        </div>
        <div className="tree">
          {(report?.files ?? []).map((f) => {
            const n = report!.issues.filter((i) => i.file === f.path).length;
            return (
              <button key={f.path} className={`tree-row ${f.path === file ? 'active' : ''}`} onClick={() => report && load(report, f.path)}>
                <Icon name="env" size={14} />
                <span className="label mono">{f.path}</span>
                <span className="meta">{n > 0 ? <span className="count">{n}</span> : f.entries.length}</span>
              </button>
            );
          })}
          {report && report.files.length === 0 && <span className="faint" style={{ padding: 8 }}>No .env files yet.</span>}
          <button
            className="btn sm dashed"
            style={{ margin: 8 }}
            onClick={async () => {
              const name = window.prompt('New env file', '.env.test');
              if (!name || !/^\.env(\.[\w-]+)?$/.test(name)) return;
              const r = await guard(api.envSave(name, []));
              if (r) {
                setFile(name);
                load(r, name);
              }
            }}
          >
            <Icon name="plus" size={13} /> New file
          </button>
        </div>
        <div className="side-foot">
          <span className="faint" style={{ fontSize: 11.5 }}>
            {report?.schemaSource ? (
              <>
                Validated against{' '}
                <button className="mono" style={{ background: 'none', border: 0, padding: 0, color: 'var(--ember)', cursor: 'pointer', fontSize: 11.5 }} onClick={() => openFile(report.schemaSource!)}>
                  {report.schemaSource}
                </button>
              </>
            ) : (
              'No Config schema found. Add EnvironmentVariables (class-validator) or a Joi validationSchema to validate.'
            )}
          </span>
        </div>
      </aside>
      <main className="page">
        <div className="page-inner" style={{ maxWidth: 1080 }}>
          <div className="row">
            <h1 className="mono">{file}</h1>
            <span className="muted" style={{ marginLeft: 8 }}>
              {report?.usages.length ?? 0} reads in code · {report?.schema.length ?? 0} schema variables
            </span>
            <button className="btn lg primary" style={{ marginLeft: 'auto' }} disabled={!dirty} onClick={() => void save()}>
              <Icon name="save" size={14} /> Save
            </button>
          </div>
          {issues.length > 0 && (
            <div className="col" style={{ gap: 8 }}>
              {issues.map((i) => (
                <div key={i.key + i.kind} className="note" style={{ alignItems: 'center' }}>
                  <Icon name={i.kind === 'unused' ? 'info' : 'warning'} size={16} style={{ color: i.kind === 'unused' ? 'var(--info)' : i.kind === 'missing' || i.kind === 'empty' ? 'var(--warning)' : 'var(--error)' }} />
                  <span style={{ flex: 1 }}>{i.message}</span>
                  {(i.kind === 'missing' || i.kind === 'unused') && (
                    <button className="btn sm" onClick={() => fixIssue(i)}>
                      {i.kind === 'missing' ? `Add ${i.key}` : 'Remove'}
                    </button>
                  )}
                </div>
              ))}
            </div>
          )}
          <table className="table">
            <thead>
              <tr>
                <th style={{ width: '32%' }}>Variable</th>
                <th>Value</th>
                <th style={{ width: 120 }}>Schema</th>
                <th style={{ width: 70 }} />
              </tr>
            </thead>
            <tbody>
              {rows.map((r, i) => {
                const s = schemaFor(r.key);
                const secret = SECRET.test(r.key) && !reveal[r.key];
                return (
                  <tr key={i} style={{ opacity: r.removed ? 0.4 : 1 }}>
                    <td>
                      <input className="input sm mono" value={r.key} aria-label="Variable name" disabled={r.original !== undefined} onChange={(e) => setRows(rows.map((x, j) => (j === i ? { ...x, key: e.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, '_') } : x)))} />
                    </td>
                    <td>
                      <div className="row">
                        <input className="input sm mono grow" type={secret ? 'password' : 'text'} value={r.value} aria-label={`Value of ${r.key}`} onChange={(e) => setRows(rows.map((x, j) => (j === i ? { ...x, value: e.target.value } : x)))} />
                        {SECRET.test(r.key) && (
                          <button className="btn sm ghost" onClick={() => setReveal((v) => ({ ...v, [r.key]: !v[r.key] }))}>
                            {reveal[r.key] ? 'Hide' : 'Show'}
                          </button>
                        )}
                      </div>
                    </td>
                    <td className="faint" style={{ fontSize: 12 }}>
                      {s ? `${s.ty}${s.required ? ' · required' : ''}` : '—'}
                    </td>
                    <td>
                      <button className="icon-btn" aria-label={r.removed ? `Restore ${r.key}` : `Remove ${r.key}`} onClick={() => setRows(r.original === undefined ? rows.filter((_, j) => j !== i) : rows.map((x, j) => (j === i ? { ...x, removed: !x.removed } : x)))}>
                        <Icon name={r.removed ? 'undo' : 'trash'} size={14} />
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          <button className="btn dashed" style={{ alignSelf: 'flex-start' }} onClick={() => setRows([...rows, { key: '', value: '' }])}>
            <Icon name="plus" size={14} /> Add variable
          </button>
          {report && report.usages.length > 0 && (
            <section className="col" style={{ gap: 8 }}>
              <span className="caps">Read in code</span>
              <div className="col" style={{ gap: 2 }}>
                {[...new Set(report.usages.map((u) => u.key))].sort().map((k) => {
                  const first = report.usages.find((u) => u.key === k)!;
                  return (
                    <button key={k} className="tree-row" onClick={() => openFile(first.file, first.line)}>
                      <span className="mono" style={{ color: 'var(--info)' }}>
                        {k}
                      </span>
                      <span className="meta mono">
                        {first.file}:{first.line}
                      </span>
                    </button>
                  );
                })}
              </div>
            </section>
          )}
        </div>
      </main>
    </>
  );
}
