import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../api';
import { Icon } from '../icons';
import { useApp, openFile, showPreview, applyChangeset, guard, toast } from '../store';
import { ContextMenu, type MenuItem } from '../components/Shell';
import { moduleMenu } from '../components/Workspace';
import type { Graph, } from '../bindings/Graph';
import type { GraphNode } from '../bindings/GraphNode';
import type { GraphEdge } from '../bindings/GraphEdge';
import type { Diagnostic } from '../bindings/Diagnostic';
import type { FixAction } from '../bindings/FixAction';

type Mode = 'imports' | 'providers' | 'exports';
type Pos = Record<string, { x: number; y: number }>;

function loadPositions(key: string): Pos {
  try {
    return JSON.parse(localStorage.getItem(key) ?? '{}');
  } catch {
    return {};
  }
}

function savePositions(key: string, pos: Pos) {
  try {
    localStorage.setItem(key, JSON.stringify(pos));
  } catch {
    /* storage unavailable */
  }
}

function edgePath(a: GraphNode, b: GraphNode): { d: string; mx: number; my: number } {
  const ax = a.x + a.w / 2;
  const bx = b.x + b.w / 2;
  if (b.y > a.y + a.h / 2) {
    const ay = a.y + a.h;
    const by = b.y;
    const midY = ay + Math.min(48, (by - ay) / 2);
    if (Math.abs(ax - bx) < 2) return { d: `M ${ax} ${ay} L ${bx} ${by - 4}`, mx: ax, my: (ay + by) / 2 };
    const r = Math.min(10, Math.abs(bx - ax) / 2, (by - ay) / 4);
    const dir = bx > ax ? 1 : -1;
    return {
      d: `M ${ax} ${ay} L ${ax} ${midY - r} Q ${ax} ${midY} ${ax + dir * r} ${midY} L ${bx - dir * r} ${midY} Q ${bx} ${midY} ${bx} ${midY + r} L ${bx} ${by - 4}`,
      mx: (ax + bx) / 2,
      my: midY,
    };
  }
  if (b.y + b.h < a.y + a.h / 2) {
    // Upward (cycle) edge: leave from the right side, curve up into the target's right side.
    const sx = a.x + a.w;
    const sy = a.y + a.h / 2;
    const tx = b.x + b.w;
    const ty = b.y + b.h / 2;
    const out = Math.max(sx, tx) + 44;
    return { d: `M ${sx} ${sy} C ${out} ${sy}, ${out} ${ty}, ${tx + 4} ${ty}`, mx: out - 10, my: (sy + ty) / 2 };
  }
  // Same row.
  const left = b.x > a.x;
  const sx = left ? a.x + a.w : a.x;
  const tx = left ? b.x : b.x + b.w;
  const y = a.y + a.h / 2;
  return { d: `M ${sx} ${y} C ${(sx + tx) / 2} ${y - 30}, ${(sx + tx) / 2} ${y - 30}, ${tx + (left ? -4 : 4)} ${y}`, mx: (sx + tx) / 2, my: y - 22 };
}

const EDGE_STYLE: Record<string, { stroke: string; dash?: string }> = {
  import: { stroke: 'var(--edge)' },
  inject: { stroke: 'var(--edge)' },
  cycle: { stroke: 'var(--warning)', dash: '6 6' },
  missing: { stroke: 'var(--error)', dash: '6 6' },
};

export function GraphView() {
  const model = useApp((s) => s.model)!;
  const [mode, setMode] = useState<Mode>('imports');
  const [showPackages, setShowPackages] = useState(true);
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<string | null>(null);
  const [view, setViewBox] = useState({ x: 0, y: 0, k: 1 });
  const posKey = `nf.graph.${model.root}.${mode}`;
  const [overrides, setOverrides] = useState<Pos>(() => loadPositions(posKey));
  const [panning, setPanning] = useState(false);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const canvas = useRef<HTMLDivElement>(null);
  const fitted = useRef(false);

  useEffect(() => setOverrides(loadPositions(posKey)), [posKey]);

  const graph: Graph = useMemo(() => {
    const base = mode === 'providers' ? model.providerGraph : model.graph;
    let nodes = base.nodes;
    let edges = base.edges;
    if (mode !== 'providers' && !showPackages) {
      const ext = new Set(nodes.filter((n) => n.kind === 'external' && n.status === 'ok').map((n) => n.id));
      nodes = nodes.filter((n) => !ext.has(n.id));
      edges = edges.filter((e) => !ext.has(e.to) && !ext.has(e.from));
    }
    if (mode === 'exports') {
      edges = edges
        .filter((e) => e.kind !== 'missing')
        .map((e) => {
          const target = model.modules.find((m) => m.name === e.to);
          const label = target ? (target.exports.length ? target.exports.join(', ') : 'exports nothing') : e.label;
          return { ...e, label };
        });
    }
    nodes = nodes.map((n) => (overrides[n.id] ? { ...n, ...overrides[n.id] } : n));
    return { ...base, nodes, edges };
  }, [model, mode, showPackages, overrides]);

  const byId = useMemo(() => new Map(graph.nodes.map((n) => [n.id, n])), [graph]);

  const fit = () => {
    const el = canvas.current;
    if (!el || graph.nodes.length === 0) return;
    const minX = Math.min(...graph.nodes.map((n) => n.x)) - 40;
    const minY = Math.min(...graph.nodes.map((n) => n.y)) - 40;
    const maxX = Math.max(...graph.nodes.map((n) => n.x + n.w)) + 80;
    const maxY = Math.max(...graph.nodes.map((n) => n.y + n.h)) + 40;
    const k = Math.min(1.2, Math.max(0.2, Math.min(el.clientWidth / (maxX - minX), el.clientHeight / (maxY - minY))));
    setViewBox({ k, x: (el.clientWidth - (maxX - minX) * k) / 2 - minX * k, y: Math.max(10, (el.clientHeight - (maxY - minY) * k) / 2) - minY * k });
  };

  useEffect(() => {
    if (!fitted.current && graph.nodes.length) {
      fitted.current = true;
      requestAnimationFrame(fit);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graph]);

  useEffect(() => {
    fitted.current = false;
    requestAnimationFrame(fit);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, showPackages]);

  const zoom = (factor: number, cx?: number, cy?: number) => {
    const el = canvas.current!;
    const px = cx ?? el.clientWidth / 2;
    const py = cy ?? el.clientHeight / 2;
    setViewBox((v) => {
      const k = Math.min(2.5, Math.max(0.15, v.k * factor));
      return { k, x: px - ((px - v.x) * k) / v.k, y: py - ((py - v.y) * k) / v.k };
    });
  };

  const onWheel = (e: React.WheelEvent) => {
    const rect = canvas.current!.getBoundingClientRect();
    if (e.ctrlKey || Math.abs(e.deltaY) > 0) zoom(e.deltaY < 0 ? 1.1 : 1 / 1.1, e.clientX - rect.left, e.clientY - rect.top);
  };

  const onCanvasDown = (e: React.MouseEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest('.graph-node')) return;
    setPanning(true);
    const start = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y };
    const move = (ev: MouseEvent) => setViewBox((v) => ({ ...v, x: start.vx + ev.clientX - start.x, y: start.vy + ev.clientY - start.y }));
    const up = () => {
      setPanning(false);
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  };

  const onNodeDown = (e: React.MouseEvent, n: GraphNode) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const start = { x: e.clientX, y: e.clientY, nx: n.x, ny: n.y };
    let moved = false;
    const move = (ev: MouseEvent) => {
      const dx = (ev.clientX - start.x) / view.k;
      const dy = (ev.clientY - start.y) / view.k;
      if (!moved && Math.abs(dx) + Math.abs(dy) < 4) return;
      moved = true;
      setOverrides((o) => ({ ...o, [n.id]: { x: Math.round(start.nx + dx), y: Math.round(start.ny + dy) } }));
    };
    const up = () => {
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
      if (moved) setOverrides((o) => (savePositions(posKey, o), o));
      else setSelected(n.id);
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  };

  const q = query.trim().toLowerCase();
  const matches = (n: GraphNode) => !q || n.label.toLowerCase().includes(q) || (mode !== 'providers' && model.modules.find((m) => m.name === n.id)?.providers.some((p) => p.token.toLowerCase().includes(q)));
  const issues = model.diagnostics;
  const errors = issues.filter((d) => d.severity !== 'info').length;
  const selectedModule = model.modules.find((m) => m.name === selected);
  const selectedClass = mode === 'providers' ? model.classes.find((c) => c.name === selected) : undefined;
  const unregistered = model.classes.filter((c) => c.modules.length === 0 && ['service', 'provider', 'gateway', 'resolver'].includes(c.kind));

  return (
    <>
      <main className="main" style={{ background: 'var(--bg-canvas)' }}>
        <div className="row" style={{ minHeight: 48, flex: 'none', padding: '8px 16px', borderBottom: '1px solid var(--border-subtle)', background: 'var(--bg-panel)', flexWrap: 'wrap', gap: 10 }}>
          <span style={{ fontSize: 14, fontWeight: 600 }}>Module graph</span>
          <div className="segmented" style={{ marginLeft: 8 }}>
            {(['imports', 'providers', 'exports'] as Mode[]).map((m) => (
              <button key={m} aria-pressed={mode === m} onClick={() => setMode(m)}>
                {m.charAt(0).toUpperCase() + m.slice(1)}
              </button>
            ))}
          </div>
          <label className="row" style={{ height: 28, padding: '0 10px', background: 'var(--bg-base)', border: '1px solid var(--border-strong)', borderRadius: 7, color: 'var(--text-2)', flex: '0 1 220px' }}>
            <Icon name="search" size={14} />
            <input
              placeholder="Find module or provider"
              aria-label="Find module or provider"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              style={{ flex: 1, minWidth: 0, background: 'none', border: 0, fontSize: 12, outline: 'none' }}
            />
          </label>
          {mode !== 'providers' && (
            <label className="check" style={{ fontSize: 12, color: 'var(--text-2)' }}>
              <input type="checkbox" checked={showPackages} onChange={(e) => setShowPackages(e.target.checked)} />
              Packages
            </label>
          )}
          <span className="row muted" style={{ marginLeft: 'auto', fontSize: 12 }}>
            <span className="dot ok" />
            Static scan · refreshed {model.scanMs < 1000 ? `${model.scanMs.toFixed(0)} ms` : `${(model.scanMs / 1000).toFixed(1)} s`} after save
          </span>
          <button className="btn sm" aria-label="Zoom out" onClick={() => zoom(1 / 1.2)}>
            −
          </button>
          <button className="btn sm" aria-label="Zoom in" onClick={() => zoom(1.2)}>
            +
          </button>
          <button className="btn sm" onClick={fit}>
            Fit
          </button>
          <button
            className="btn sm ghost"
            onClick={() => {
              setOverrides({});
              savePositions(posKey, {});
            }}
          >
            Reset layout
          </button>
        </div>
        <div className={`graph-canvas ${panning ? 'panning' : ''}`} ref={canvas} onMouseDown={onCanvasDown} onWheel={onWheel}>
          {graph.nodes.length === 0 && (
            <div className="empty">
              <Icon name="graph" size={28} />
              <span>{mode === 'providers' ? 'No providers yet.' : 'No modules found. Is this a Nest project?'}</span>
            </div>
          )}
          <div style={{ position: 'absolute', left: 0, top: 0, transform: `translate(${view.x}px, ${view.y}px) scale(${view.k})`, transformOrigin: '0 0' }}>
            <svg style={{ position: 'absolute', left: 0, top: 0, overflow: 'visible', width: 1, height: 1 }} aria-hidden="true">
              <defs>
                {Object.entries(EDGE_STYLE).map(([k, s]) => (
                  <marker key={k} id={`arrow-${k}`} viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                    <path d="M0 0 L10 5 L0 10 z" fill={s.stroke} />
                  </marker>
                ))}
              </defs>
              {graph.edges.map((e: GraphEdge, i) => {
                const a = byId.get(e.from);
                const b = byId.get(e.to);
                if (!a || !b) return null;
                const { d, mx, my } = edgePath(a, b);
                const st = EDGE_STYLE[e.kind] ?? EDGE_STYLE.import;
                const dim = q && !(matches(a) || matches(b));
                const hl = selected && (e.from === selected || e.to === selected);
                return (
                  <g key={i} opacity={dim ? 0.15 : 1}>
                    <path d={d} fill="none" stroke={st.stroke} strokeWidth={hl ? 2.6 : 1.8} strokeDasharray={e.forwardRef ? '2 4' : st.dash} markerEnd={`url(#arrow-${e.kind})`} />
                    {e.label && mode === 'exports' && (
                      <text x={mx} y={my - 4} fontSize="11" fill="var(--text-2)" textAnchor="middle" style={{ paintOrder: 'stroke', stroke: 'var(--bg-canvas)', strokeWidth: 4 }}>
                        {e.label.length > 32 ? `${e.label.slice(0, 30)}…` : e.label}
                      </text>
                    )}
                    {e.kind === 'cycle' && b.y < a.y && (
                      <text x={mx + 16} y={my} fontSize="12" fill="var(--warning)">
                        cycle
                      </text>
                    )}
                  </g>
                );
              })}
            </svg>
            {graph.nodes.map((n) => {
              const cls = ['graph-node', n.root ? 'root' : '', n.kind === 'external' ? 'external' : '', selected === n.id ? 'selected' : n.status !== 'ok' ? n.status : '', q && !matches(n) ? 'dim' : '', dropTarget === n.id ? 'drop-target' : ''].join(' ');
              const iconColor = n.kind === 'controller' ? 'var(--c-controller)' : n.kind === 'provider' ? 'var(--c-service)' : n.status === 'orphan' ? 'var(--error)' : 'var(--c-module)';
              return (
                <div
                  key={n.id}
                  className={cls}
                  style={{ left: n.x, top: n.y, width: n.w, height: n.h }}
                  onMouseDown={(e) => onNodeDown(e, n)}
                  onDoubleClick={() => n.file && openFile(n.file, n.line)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    const m = model.modules.find((x) => x.name === n.id);
                    if (m) setMenu({ x: e.clientX, y: e.clientY, items: moduleMenu(m) });
                  }}
                  onDragOver={(e) => {
                    if (n.kind === 'module' && e.dataTransfer.types.includes('application/x-nf-provider')) {
                      e.preventDefault();
                      setDropTarget(n.id);
                    }
                  }}
                  onDragLeave={() => setDropTarget((t) => (t === n.id ? null : t))}
                  onDrop={(e) => {
                    e.preventDefault();
                    setDropTarget(null);
                    const provider = e.dataTransfer.getData('application/x-nf-provider');
                    if (provider) void guard(api.previewWire(provider, n.id)).then((cs) => cs && showPreview(cs, { title: `Wire ${provider} into ${n.id}` }));
                  }}
                  title={n.file ?? n.group ?? n.label}
                >
                  <Icon name={n.kind === 'module' || n.kind === 'external' ? 'module' : n.kind === 'controller' ? 'api' : 'generate'} size={16} style={{ color: iconColor }} />
                  <span style={{ minWidth: 0 }}>
                    <b>{n.label}</b>
                    <span className="sub">{n.sublabel}</span>
                  </span>
                </div>
              );
            })}
          </div>
        </div>
        <div className="legend-row">
          <span>
            <span style={{ width: 24, height: 2, background: 'var(--edge)' }} />
            {mode === 'providers' ? 'injects' : 'imports'}
          </span>
          <span>
            <span style={{ width: 24, borderTop: '2px dashed var(--warning)' }} />
            circular import
          </span>
          <span>
            <span style={{ width: 24, borderTop: '2px dashed var(--error)' }} />
            needed, not wired
          </span>
          <span>
            <span style={{ width: 24, borderTop: '2px dotted var(--edge)' }} />
            forwardRef
          </span>
          <span>Drag a provider onto a module to wire it · double-click to open</span>
        </div>
      </main>
      <aside className="side right scroll" aria-label="Issues" style={{ maxWidth: 340, flexBasis: 300 }}>
        <div className="row" style={{ padding: 16, borderBottom: '1px solid var(--border-subtle)' }}>
          <span className="caps">Issues</span>
          {errors > 0 && <span className="count">{errors}</span>}
          <span className="faint" style={{ marginLeft: 'auto', fontSize: 11.5 }}>
            caught before runtime
          </span>
        </div>
        {issues.length === 0 && (
          <div className="issue">
            <span className="row" style={{ color: 'var(--success)' }}>
              <Icon name="check" size={16} /> No wiring issues
            </span>
            <span className="muted">Every injected provider is reachable and there are no circular imports.</span>
          </div>
        )}
        {issues.map((d) => (
          <IssueCard key={d.id} d={d} onSelect={(m) => setSelected(m)} />
        ))}
        {selectedModule && <ModuleDetails name={selectedModule.name} />}
        {selectedClass && (
          <section className="section">
            <span className="caps">Selected · {selectedClass.name}</span>
            {selectedClass.injects.length === 0 && <span className="muted">Injects nothing.</span>}
            {selectedClass.injects.map((i) => (
              <div key={i.param} className="row" style={{ fontSize: 12.5 }}>
                <Icon name={i.status === 'resolved' ? 'check' : i.status === 'missing' ? 'error' : 'info'} size={14} style={{ color: i.status === 'resolved' ? 'var(--success)' : i.status === 'missing' ? 'var(--error)' : 'var(--text-3)' }} />
                <span className="mono">{i.token}</span>
                <span className="faint" style={{ marginLeft: 'auto' }}>
                  {i.via ?? ''}
                </span>
              </div>
            ))}
            <button className="btn" onClick={() => openFile(selectedClass.file, selectedClass.line)}>
              Open {selectedClass.file.split('/').pop()}
            </button>
          </section>
        )}
        {mode !== 'providers' && unregistered.length > 0 && (
          <section className="section">
            <span className="caps">Not registered</span>
            <span className="faint" style={{ fontSize: 12 }}>
              Drag onto a module to add it to its providers.
            </span>
            {unregistered.map((c) => (
              <ProviderChip key={c.name} name={c.name} />
            ))}
          </section>
        )}
      </aside>
      {menu && <ContextMenu {...menu} onClose={() => setMenu(null)} />}
    </>
  );
}

function ProviderChip({ name }: { name: string }) {
  return (
    <div
      className="provider-chip"
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData('application/x-nf-provider', name);
        e.dataTransfer.effectAllowed = 'link';
      }}
      title={`Drag ${name} onto a module`}
    >
      <span className="kind service">S</span>
      {name}
    </div>
  );
}

function ModuleDetails({ name }: { name: string }) {
  const model = useApp((s) => s.model)!;
  const m = model.modules.find((x) => x.name === name);
  if (!m) return null;
  return (
    <section className="section" style={{ borderBottom: 0 }}>
      <span className="caps">Selected · {m.name}</span>
      <div className="kv">
        <span>imports</span>
        <span>{m.imports.map((i) => (i.forwardRef ? `forwardRef(${i.name})` : i.display)).join(', ') || '—'}</span>
        <span>providers</span>
        <span>{m.providers.map((p) => p.token).join(', ') || '—'}</span>
        <span>controllers</span>
        <span>{m.controllers.join(', ') || '—'}</span>
        <span>exports</span>
        <span>{m.exports.join(', ') || '—'}</span>
      </div>
      {m.providers.length > 0 && (
        <>
          <span className="faint" style={{ fontSize: 12 }}>
            Drag a provider onto another module to share it.
          </span>
          {m.providers.map((p) => (
            <ProviderChip key={p.token} name={p.token} />
          ))}
        </>
      )}
      <button className="btn" onClick={() => openFile(m.file, m.line)}>
        Open {m.file.split('/').pop()}
      </button>
    </section>
  );
}

function IssueCard({ d, onSelect }: { d: Diagnostic; onSelect: (module: string) => void }) {
  const [busy, setBusy] = useState(false);
  const color = d.severity === 'error' ? 'var(--error)' : d.severity === 'warning' ? 'var(--warning)' : 'var(--info)';
  const fix = d.fixes[0];
  const preview = async (action: FixAction) => guard(api.previewFix(action));
  return (
    <article className="issue" onMouseEnter={() => d.modules[0] && onSelect(d.modules[0])}>
      <div className="row">
        <Icon name={d.severity === 'error' ? 'error' : d.severity === 'warning' ? 'warning' : 'info'} size={16} style={{ color }} />
        <span style={{ fontWeight: 600 }}>{d.title}</span>
      </div>
      <span className="mono" style={{ fontSize: 12 }}>
        {d.subject}
      </span>
      <span className="muted" style={{ lineHeight: '19px' }}>
        {d.detail}
      </span>
      <div className="row" style={{ flexWrap: 'wrap' }}>
        {fix && (
          <button
            className="btn sm primary"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              const cs = await preview(fix.action);
              if (cs) await applyChangeset(cs);
              setBusy(false);
            }}
          >
            {fix.label}
          </button>
        )}
        {fix && (
          <button
            className="btn sm"
            onClick={async () => {
              const cs = await preview(fix.action);
              if (cs) showPreview(cs, { title: fix.label });
            }}
          >
            Preview diff
          </button>
        )}
        {d.file && (
          <button className="btn sm" onClick={() => openFile(d.file!, d.line)}>
            Open file
          </button>
        )}
        {!fix && <span className="faint" style={{ fontSize: 12 }}>Fix by hand; NestFactory rechecks on save.</span>}
      </div>
    </article>
  );
}

export function useGraphToast() {
  useEffect(() => {
    return useApp.subscribe((s, prev) => {
      if (!prev.model || !s.model || prev.model.root !== s.model.root) return;
      const before = prev.model.diagnostics.filter((d) => d.severity === 'error').length;
      const after = s.model.diagnostics.filter((d) => d.severity === 'error').length;
      if (after > before) toast(`${after - before} new wiring error${after - before === 1 ? '' : 's'} — see the module graph`, 'error');
    });
  }, []);
}
