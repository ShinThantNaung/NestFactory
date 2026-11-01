import { useEffect, useMemo, useState } from 'react';
import { api } from '../api';
import { Icon, Play } from '../icons';
import { useApp, guard, startScript, openFile, toast } from '../store';
import { timeAgo } from '../util';
import type { Endpoint } from '../bindings/Endpoint';
import type { ApiRequest } from '../bindings/ApiRequest';
import type { ApiResponse } from '../bindings/ApiResponse';
import type { HistoryEntry } from '../bindings/HistoryEntry';

const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];

function blank(port: number): ApiRequest {
  return { id: '', name: '', method: 'GET', url: `http://localhost:${port}/`, headers: [['Accept', 'application/json']], body: '' };
}

function highlightJson(text: string) {
  const esc = text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  return esc.replace(/("(\\u[a-fA-F0-9]{4}|\\[^u]|[^\\"])*"(\s*:)?|\b(true|false|null)\b|-?\d+(?:\.\d*)?(?:[eE][+-]?\d+)?)/g, (m) => {
    let color = 'var(--info)';
    if (m.startsWith('"')) color = m.endsWith(':') ? 'var(--info)' : 'var(--success)';
    else if (/true|false|null/.test(m)) color = 'var(--c-dto)';
    return `<span style="color:${color}">${m}</span>`;
  });
}

export function ApiView() {
  const model = useApp((s) => s.model)!;
  const draft = useApp((s) => s.apiDraft);
  const running = useApp((s) => s.running.run);
  const [endpoints, setEndpoints] = useState<Endpoint[]>([]);
  const [saved, setSaved] = useState<ApiRequest[]>([]);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [req, setReq] = useState<ApiRequest>(() => draft ?? blank(model.port));
  const [res, setRes] = useState<ApiResponse | null>(null);
  const [sending, setSending] = useState(false);
  const [reqTab, setReqTab] = useState<'body' | 'headers'>('body');
  const [resTab, setResTab] = useState<'body' | 'headers'>('body');
  const [filter, setFilter] = useState('');

  const refresh = () => {
    api.apiEndpoints().then(setEndpoints, () => {});
    api.apiSaved().then(setSaved, () => {});
    api.apiHistory().then(setHistory, () => {});
  };
  useEffect(refresh, [model, running]);
  useEffect(() => {
    if (draft) {
      setReq(draft);
      setRes(null);
      useApp.setState({ apiDraft: null });
    }
  }, [draft]);

  const groups = useMemo(() => {
    const out = new Map<string, Endpoint[]>();
    const f = filter.toLowerCase();
    for (const e of endpoints) {
      if (f && !`${e.method} ${e.path} ${e.summary ?? ''}`.toLowerCase().includes(f)) continue;
      const k = e.tag ?? 'Other';
      out.set(k, [...(out.get(k) ?? []), e]);
    }
    return [...out.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [endpoints, filter]);

  const load = (e: Endpoint) => {
    const url = `http://localhost:${model.port}${e.path.replace(/:(\w+)/g, '1')}`;
    const body = e.method === 'GET' || e.method === 'DELETE' ? '' : (e.bodyExample ?? '{\n  \n}');
    setReq({ id: '', name: `${e.method} ${e.path}`, method: e.method, url, headers: [['Accept', 'application/json']], body });
    setRes(null);
  };

  const send = async () => {
    setSending(true);
    try {
      setRes(await api.apiSend(req));
      api.apiHistory().then(setHistory, () => {});
    } catch (e) {
      toast(String(e), 'error');
    } finally {
      setSending(false);
    }
  };

  const save = async () => {
    const name = window.prompt('Save request as', req.name || `${req.method} ${new URL(req.url, 'http://x').pathname}`);
    if (!name) return;
    const list = await guard(api.apiSave({ ...req, name }));
    if (list) {
      setSaved(list);
      const latest = list.find((r) => r.name === name);
      if (latest) setReq(latest);
      toast(`Saved “${name}”`, 'success');
    }
  };

  const statusColor = !res ? '' : res.error ? 'var(--error)' : res.status < 300 ? 'var(--success)' : res.status < 400 ? 'var(--info)' : res.status < 500 ? 'var(--warning)' : 'var(--error)';
  const contentType = res?.headers.find(([k]) => k.toLowerCase() === 'content-type')?.[1] ?? '';
  return (
    <>
      <aside className="side" aria-label="Endpoints" style={{ maxWidth: 320, flexBasis: 290 }}>
        <div className="side-head">
          <span className="caps">Endpoints</span>
          <button className="icon-btn" aria-label="Refresh" title="Refresh" onClick={refresh}>
            <Icon name="restart" size={14} />
          </button>
        </div>
        <div style={{ padding: '8px 10px 0' }}>
          <input className="input sm" style={{ width: '100%' }} placeholder="Filter" aria-label="Filter endpoints" value={filter} onChange={(e) => setFilter(e.target.value)} />
        </div>
        <div className="scroll grow">
          <div className="tree">
            {groups.length === 0 && <div className="empty" style={{ height: 'auto' }}>No routes found. Controllers with @Get/@Post appear here.</div>}
            {groups.map(([tag, list]) => (
              <div key={tag}>
                <div className="caps" style={{ padding: '10px 8px 4px', fontSize: 10.5 }}>
                  {tag}
                </div>
                {list.map((e) => (
                  <button key={`${e.method}${e.path}`} className="tree-row" onClick={() => load(e)} onDoubleClick={() => e.file && openFile(e.file, e.line)} title={e.summary ?? ''}>
                    <span className={`method m-${e.method}`}>{e.method}</span>
                    <span className="label mono" style={{ fontSize: 12 }}>
                      {e.path}
                    </span>
                    {e.source !== 'code' && (
                      <span className="meta" title="From the running app's Swagger spec">
                        live
                      </span>
                    )}
                  </button>
                ))}
              </div>
            ))}
            {saved.length > 0 && (
              <>
                <div className="caps" style={{ padding: '14px 8px 4px', fontSize: 10.5 }}>
                  Saved
                </div>
                {saved.map((r) => (
                  <div key={r.id} className="row">
                    <button className="tree-row grow" onClick={() => { setReq(r); setRes(null); }}>
                      <span className={`method m-${r.method}`}>{r.method}</span>
                      <span className="label">{r.name}</span>
                    </button>
                    <button className="icon-btn" aria-label={`Delete ${r.name}`} onClick={() => void guard(api.apiDelete(r.id)).then((l) => l && setSaved(l))}>
                      <Icon name="trash" size={13} />
                    </button>
                  </div>
                ))}
              </>
            )}
            {history.length > 0 && (
              <>
                <div className="caps" style={{ padding: '14px 8px 4px', fontSize: 10.5 }}>
                  History
                </div>
                {history.slice(0, 15).map((h, i) => (
                  <button key={i} className="tree-row" onClick={() => { setReq(h.request); setRes(null); }}>
                    <span className={`method m-${h.request.method}`}>{h.request.method}</span>
                    <span className="label mono" style={{ fontSize: 12 }}>
                      {h.request.url.replace(/^https?:\/\/[^/]+/, '')}
                    </span>
                    <span className="meta" style={{ color: h.status >= 400 || h.status === 0 ? 'var(--error)' : undefined }}>
                      {h.status || 'err'} · {timeAgo(h.at)}
                    </span>
                  </button>
                ))}
              </>
            )}
          </div>
        </div>
      </aside>
      <main className="main">
        <form
          className="row"
          style={{ padding: 16, borderBottom: '1px solid var(--border-subtle)', gap: 8 }}
          onSubmit={(e) => {
            e.preventDefault();
            void send();
          }}
        >
          <select className="input" style={{ width: 110 }} value={req.method} aria-label="Method" onChange={(e) => setReq({ ...req, method: e.target.value })}>
            {METHODS.map((m) => (
              <option key={m}>{m}</option>
            ))}
          </select>
          <input className="input mono grow" value={req.url} aria-label="URL" onChange={(e) => setReq({ ...req, url: e.target.value })} spellCheck={false} />
          <button className="btn lg primary" type="submit" disabled={sending}>
            <Icon name="send" size={14} /> {sending ? 'Sending…' : 'Send'}
          </button>
          <button className="btn lg" type="button" onClick={() => void save()}>
            <Icon name="save" size={14} /> Save
          </button>
        </form>
        {!running && (
          <div className="note" style={{ margin: '12px 16px 0' }}>
            <Icon name="info" size={16} />
            <span style={{ flex: 1 }}>The app isn't running from NestFactory. Start it to send requests and read live endpoints from Swagger.</span>
            <button className="btn sm primary" onClick={() => void startScript()}>
              <Play size={10} /> Start
            </button>
          </div>
        )}
        <div className="split-v" style={{ flexDirection: 'column', flexWrap: 'nowrap' }}>
          <section style={{ flex: '0 0 auto', maxHeight: '45%', display: 'flex', flexDirection: 'column', borderBottom: '1px solid var(--border-subtle)' }}>
            <div className="panel-tabs" style={{ background: 'var(--bg-panel)' }} role="tablist">
              <button role="tab" aria-selected={reqTab === 'body'} onClick={() => setReqTab('body')}>
                Body
              </button>
              <button role="tab" aria-selected={reqTab === 'headers'} onClick={() => setReqTab('headers')}>
                Headers {req.headers.length > 0 && <span className="faint">{req.headers.length}</span>}
              </button>
            </div>
            {reqTab === 'body' ? (
              <textarea
                className="mono"
                value={req.body}
                aria-label="Request body"
                placeholder={req.method === 'GET' ? 'GET requests have no body' : '{ "name": "value" }'}
                onChange={(e) => setReq({ ...req, body: e.target.value })}
                spellCheck={false}
                style={{ minHeight: 140, resize: 'vertical', background: 'var(--bg-editor)', border: 0, padding: '12px 16px', outline: 'none', fontSize: 12.5, lineHeight: '20px' }}
              />
            ) : (
              <div className="col" style={{ padding: 12, gap: 6, overflow: 'auto' }}>
                {req.headers.map(([k, v], i) => (
                  <div key={i} className="row">
                    <input className="input sm mono" style={{ width: 220 }} value={k} aria-label="Header name" onChange={(e) => setReq({ ...req, headers: req.headers.map((h, j) => (j === i ? [e.target.value, h[1]] : h)) })} />
                    <input className="input sm mono grow" value={v} aria-label="Header value" onChange={(e) => setReq({ ...req, headers: req.headers.map((h, j) => (j === i ? [h[0], e.target.value] : h)) })} />
                    <button className="icon-btn" aria-label="Remove header" onClick={() => setReq({ ...req, headers: req.headers.filter((_, j) => j !== i) })}>
                      <Icon name="close" size={13} />
                    </button>
                  </div>
                ))}
                <button className="btn sm dashed" style={{ alignSelf: 'flex-start' }} onClick={() => setReq({ ...req, headers: [...req.headers, ['', '']] })}>
                  <Icon name="plus" size={13} /> Add header
                </button>
              </div>
            )}
          </section>
          <section style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <div className="panel-tabs" role="tablist" style={{ background: 'var(--bg-panel)' }}>
              <button role="tab" aria-selected={resTab === 'body'} onClick={() => setResTab('body')}>
                Response
              </button>
              <button role="tab" aria-selected={resTab === 'headers'} onClick={() => setResTab('headers')}>
                Headers
              </button>
              {res && (
                <span className="row" style={{ marginLeft: 'auto', fontSize: 12 }}>
                  <span style={{ color: statusColor, fontWeight: 600 }}>{res.error ? 'Failed' : `${res.status} ${res.statusText}`}</span>
                  <span className="faint">{Math.round(res.timeMs)} ms</span>
                  <span className="faint">{res.size < 1024 ? `${res.size} B` : `${(res.size / 1024).toFixed(1)} KB`}</span>
                </span>
              )}
            </div>
            <div className="log" style={{ background: 'var(--bg-editor)', color: 'var(--text)' }}>
              {!res && <div className="faint">Send a request to see the response.</div>}
              {res?.error && <div style={{ color: 'var(--error)' }}>{res.error}</div>}
              {res && !res.error && resTab === 'body' && (contentType.includes('json') ? <div dangerouslySetInnerHTML={{ __html: highlightJson(res.body) }} /> : <div>{res.body}</div>)}
              {res && resTab === 'headers' && res.headers.map(([k, v]) => (
                <div key={k}>
                  <span style={{ color: 'var(--info)' }}>{k}</span>: {v}
                </div>
              ))}
            </div>
          </section>
        </div>
      </main>
    </>
  );
}
