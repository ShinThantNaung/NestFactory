import { useEffect, useState } from 'react';
import { api } from '../api';
import { Icon } from '../icons';
import { useApp, applyChangeset } from '../store';
import { basename, dirname } from '../util';
import { DiffView } from './Editor';
import type { Changeset } from '../bindings/Changeset';

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

