import { useEffect, useMemo, useState } from 'react';
import { api } from '../api';
import { Icon, Play } from '../icons';
import { useApp, runTests, openFile, startDebug } from '../store';
import { EditorArea } from '../components/Editor';
import { basename } from '../util';
import type { TestSuite } from '../bindings/TestSuite';
import type { TestCase } from '../bindings/TestCase';
import type { Variable } from '../bindings/Variable';

function StatusIcon({ status, size = 14 }: { status: string; size?: number }) {
  if (status === 'passed') return <Icon name="check" size={size} style={{ color: 'var(--success)' }} />;
  if (status === 'failed') return <Icon name="close" size={size} style={{ color: 'var(--error)' }} />;
  return <Icon name="minimize" size={size} style={{ color: 'var(--text-3)' }} />;
}

function suiteName(s: { file: string; tests: TestCase[] }) {
  return s.tests[0]?.ancestors[0] ?? basename(s.file).replace(/\.(e2e-)?spec\.ts$/, '');
}

function VariableTree({ objectId, depth = 0 }: { objectId: string; depth?: number }) {
  const [vars, setVars] = useState<Variable[] | null>(null);
  const [open, setOpen] = useState<Record<string, boolean>>({});
  useEffect(() => {
    api.debugProperties(objectId).then(setVars, () => setVars([]));
  }, [objectId]);
  if (!vars) return <div className="faint" style={{ paddingLeft: depth * 14 }}>…</div>;
  return (
    <>
      {vars.slice(0, 200).map((v) => (
        <div key={v.name}>
          <button
            className="row mono"
            style={{ background: 'none', border: 0, padding: `1px 0 1px ${depth * 14}px`, cursor: v.objectId ? 'pointer' : 'default', fontSize: 12, textAlign: 'left', width: '100%' }}
            onClick={() => v.objectId && setOpen((o) => ({ ...o, [v.name]: !o[v.name] }))}
          >
            {v.objectId ? <Icon name={open[v.name] ? 'chevronDown' : 'chevronRight'} size={11} /> : <span style={{ width: 11 }} />}
            <span style={{ color: 'var(--c-controller)', opacity: 0.85 }}>{v.name}</span>:
            <span style={{ color: v.ty === 'string' ? 'var(--success)' : v.ty === 'number' ? 'var(--info)' : 'var(--text-2)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{v.value}</span>
          </button>
          {v.objectId && open[v.name] && depth < 5 && <VariableTree objectId={v.objectId} depth={depth + 1} />}
        </div>
      ))}
    </>
  );
}

function DebugPanel({ failing }: { failing: { suite: TestSuite; test: TestCase } | null }) {
  const debug = useApp((s) => s.debug);
  const frames = debug.frames;
  const [frameIdx, setFrameIdx] = useState(0);
  useEffect(() => setFrameIdx(0), [frames]);
  const frame = frames?.[frameIdx];
  const visible = frames?.filter((f) => !f.external) ?? [];
  const hidden = (frames?.length ?? 0) - visible.length;
  return (
    <section aria-label="Debug panel" style={{ height: 250, flex: 'none', display: 'flex', flexWrap: 'wrap', background: 'var(--bg-panel)', borderTop: '1px solid var(--border-subtle)', fontSize: 12.5, overflow: 'hidden' }}>
      <div style={{ flex: '1 1 240px', padding: '12px 14px', borderRight: '1px solid var(--border-subtle)', display: 'flex', flexDirection: 'column', gap: 6, minWidth: 0, overflow: 'auto', height: '100%' }}>
        <span className="caps">Variables</span>
        {!frame && <span className="faint">{debug.state === 'idle' || debug.state === 'ended' ? 'Start debugging to inspect variables.' : 'Running — set a breakpoint in the gutter (F9).'}</span>}
        {frame?.scopes.filter((s) => s.objectId).slice(0, 3).map((s) => (
          <div key={s.objectId}>
            <div className="faint" style={{ fontSize: 11, margin: '2px 0' }}>
              {s.type === 'local' ? 'Local' : s.type === 'block' ? 'Block' : s.type === 'closure' ? `Closure${s.name ? ` (${s.name})` : ''}` : s.type}
            </div>
            <VariableTree objectId={s.objectId} />
          </div>
        ))}
      </div>
      <div style={{ flex: '1 1 200px', padding: '12px 14px', borderRight: '1px solid var(--border-subtle)', display: 'flex', flexDirection: 'column', gap: 6, minWidth: 0, overflow: 'auto', height: '100%' }}>
        <span className="caps">Call stack</span>
        {!frames && <span className="faint">Not paused.</span>}
        {frames?.map((f, i) =>
          f.external ? null : (
            <button
              key={i}
              className="row"
              style={{ padding: '3px 6px', background: i === frameIdx ? 'var(--bg-selected)' : 'none', border: 0, borderRadius: 4, cursor: 'pointer', textAlign: 'left', fontSize: 12 }}
              onClick={() => {
                setFrameIdx(i);
                if (f.file) openFile(f.file, f.line);
              }}
            >
              {f.function} <span className="faint">{f.file ? `${basename(f.file)}:${f.line}` : ''}</span>
            </button>
          ),
        )}
        {hidden > 0 && <span className="faint" style={{ padding: '3px 6px', fontSize: 12 }}>node internals · {hidden} frames hidden</span>}
      </div>
      <div style={{ flex: '2 1 300px', padding: '12px 14px', display: 'flex', flexDirection: 'column', gap: 8, minWidth: 0, height: '100%' }}>
        <span className="caps">Test output</span>
        {failing ? (
          <>
            <div className="mono selectable" style={{ fontSize: 12, lineHeight: '19px', whiteSpace: 'pre-wrap', overflow: 'auto', flex: 1 }}>
              <span style={{ color: 'var(--error)' }}>
                ✕ {failing.test.ancestors.join(' › ')} › {failing.test.title}
              </span>
              {'\n\n'}
              {failing.test.failure?.split('\n').slice(0, 14).join('\n')}
            </div>
            <div className="row" style={{ flexWrap: 'wrap' }}>
              <button className="btn sm" onClick={() => void runTests({ file: failing.suite.file, name: failing.test.title, coverage: false })}>
                Rerun failed
              </button>
              <button className="btn sm" onClick={() => openFile(failing.suite.file, failing.test.line)}>
                Open spec
              </button>
              <button className="btn sm" onClick={() => void startDebug('test', failing.suite.file)}>
                Debug this file
              </button>
            </div>
          </>
        ) : (
          <span className="faint">No failing tests.</span>
        )}
      </div>
    </section>
  );
}

export function TestsView() {
  const model = useApp((s) => s.model)!;
  const run = useApp((s) => s.testRun);
  const busy = useApp((s) => s.testsRunning);
  const debug = useApp((s) => s.debug);
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const [selected, setSelected] = useState<{ file: string; title: string } | null>(null);

  const suites = useMemo(() => {
    const byFile = new Map<string, TestSuite>();
    for (const s of run?.suites ?? []) byFile.set(s.file, s);
    for (const f of model.files.filter((x) => x.kind === 'spec')) {
      if (!byFile.has(f.path)) byFile.set(f.path, { file: f.path, e2e: f.path.includes('e2e'), status: 'skipped', tests: [], message: null });
    }
    return [...byFile.values()].sort((a, b) => a.file.localeCompare(b.file));
  }, [run, model]);

  const failing = useMemo(() => {
    for (const s of run?.suites ?? []) {
      for (const t of s.tests) if (t.status === 'failed' && (!selected || (selected.file === s.file && selected.title === t.title))) return { suite: s, test: t };
    }
    for (const s of run?.suites ?? []) for (const t of s.tests) if (t.status === 'failed') return { suite: s, test: t };
    return null;
  }, [run, selected]);

  const total = run ? run.passed + run.failed : 0;
  const debugging = !['idle', 'ended'].includes(debug.state);
  const unit = suites.filter((s) => !s.e2e);
  const e2e = suites.filter((s) => s.e2e);

  const renderSuite = (s: TestSuite) => {
    const isOpen = open[s.file] ?? s.status === 'failed';
    return (
      <div key={s.file}>
        <div className="row">
          <button className="tree-row grow" onClick={() => setOpen((o) => ({ ...o, [s.file]: !isOpen }))} onDoubleClick={() => openFile(s.file)} title={s.file}>
            <StatusIcon status={s.status} />
            <span className="label">{suiteName(s)}</span>
            <span className="meta">{s.tests.length || ''}</span>
          </button>
          <button className="icon-btn" title="Run this file" aria-label={`Run ${basename(s.file)}`} onClick={() => void runTests({ e2e: s.e2e, file: s.file, coverage: false })}>
            <Play size={10} />
          </button>
          <button className="icon-btn" title="Debug this file" aria-label={`Debug ${basename(s.file)}`} onClick={() => void startDebug('test', s.file)}>
            <Icon name="bug" size={13} />
          </button>
        </div>
        {isOpen && s.message && (
          <div className="mono" style={{ padding: '4px 8px 8px 30px', fontSize: 11.5, color: 'var(--error)', whiteSpace: 'pre-wrap' }}>
            {s.message.split('\n').slice(0, 8).join('\n')}
          </div>
        )}
        {isOpen &&
          s.tests.map((t) => (
            <button
              key={t.fullName + t.line}
              className="tree-row"
              style={{ paddingLeft: 30, color: t.status === 'failed' ? 'var(--text)' : 'var(--text-2)', background: failing?.test === t ? 'var(--error-bg)' : undefined }}
              onClick={() => {
                setSelected({ file: s.file, title: t.title });
                openFile(s.file, t.line || undefined);
              }}
            >
              <StatusIcon status={t.status} size={13} />
              <span className="label">{t.title}</span>
            </button>
          ))}
      </div>
    );
  };

  return (
    <>
      <aside className="side" aria-label="Test explorer" style={{ maxWidth: 300 }}>
        <div className="col" style={{ padding: '12px 14px', borderBottom: '1px solid var(--border-subtle)', gap: 10 }}>
          <div className="row" style={{ justifyContent: 'space-between' }}>
            <span className="caps">Tests · Jest</span>
            <button className="btn sm primary" disabled={busy} onClick={() => void runTests()}>
              <Play size={10} /> {busy ? 'Running…' : 'Run all'}
            </button>
          </div>
          {run ? (
            <>
              <div className="row" style={{ gap: 14, fontSize: 12 }}>
                <span style={{ color: 'var(--success)' }}>{run.passed} passed</span>
                {run.failed > 0 && <span style={{ color: 'var(--error)' }}>{run.failed} failed</span>}
                {run.coverage && <span className="muted">coverage {run.coverage.linesPct}%</span>}
              </div>
              <div style={{ height: 4, borderRadius: 2, background: 'var(--border-strong)', display: 'flex', overflow: 'hidden' }}>
                <span style={{ width: `${total ? (run.passed / total) * 100 : 0}%`, background: 'var(--success)' }} />
                <span style={{ width: `${total ? (run.failed / total) * 100 : 0}%`, background: 'var(--error)' }} />
              </div>
            </>
          ) : (
            <span className="faint" style={{ fontSize: 12 }}>
              {busy ? 'Running Jest…' : 'Run the suite to see results, coverage and inline pass/fail marks.'}
            </span>
          )}
          {run?.error && (
            <span className="mono" style={{ fontSize: 11.5, color: 'var(--error)', whiteSpace: 'pre-wrap' }}>
              {run.error}
            </span>
          )}
        </div>
        <div className="scroll grow">
          <div className="tree">
            <span className="caps" style={{ padding: '6px 8px 4px', fontSize: 10.5 }}>
              Unit
            </span>
            {unit.length === 0 && <span className="faint" style={{ padding: '0 8px' }}>No spec files.</span>}
            {unit.map(renderSuite)}
            <div className="row" style={{ padding: '12px 8px 4px' }}>
              <span className="caps" style={{ fontSize: 10.5 }}>
                E2E
              </span>
              <button className="btn sm ghost" style={{ marginLeft: 'auto' }} disabled={busy} onClick={() => void runTests({ e2e: true, coverage: false })}>
                Run e2e
              </button>
            </div>
            {e2e.map(renderSuite)}
          </div>
        </div>
      </aside>
      <main className="main">
        <div className="row" style={{ height: 36, flex: 'none', padding: '0 16px', background: 'var(--bg-panel)', borderBottom: '1px solid var(--border-subtle)', fontSize: 11.5, color: 'var(--text-2)', gap: 12 }}>
          <span>Click the gutter (or F9) to set breakpoints.</span>
          <span className="row" style={{ marginLeft: 'auto', gap: 12 }}>
            <span className="row" style={{ gap: 6 }}>
              <span style={{ width: 3, height: 12, background: 'var(--success)' }} />
              covered
            </span>
            <span className="row" style={{ gap: 6 }}>
              <span style={{ width: 3, height: 12, background: 'var(--error)' }} />
              not covered
            </span>
          </span>
        </div>
        <EditorArea showCoverage />
        {(debugging || failing) && <DebugPanel failing={failing} />}
      </main>
    </>
  );
}
