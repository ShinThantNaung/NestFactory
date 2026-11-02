import { useEffect, useRef, useState } from 'react';
import { Icon } from '../icons';
import { useApp, setView } from '../store';
import { attachTerminal, closeTerminal, clearTerminal, createTerminal, ensureTerminal, fitTerminal, focusTerminal, parkTerminals, terminalProfiles } from '../terminal';
import { ContextMenu, type MenuItem } from './Shell';
import type { TerminalInfo } from '../store';

/** "PowerShell", "PowerShell (2)": numbers repeated names in open order. */
function labels(terminals: TerminalInfo[]) {
  const seen: Record<string, number> = {};
  return terminals.map((t) => {
    seen[t.name] = (seen[t.name] ?? 0) + 1;
    return seen[t.name] > 1 ? `${t.name} (${seen[t.name]})` : t.name;
  });
}

async function profileMenu(): Promise<MenuItem[]> {
  const profiles = await terminalProfiles().catch(() => []);
  const def = useApp.getState().settings?.terminalProfile || profiles[0]?.id;
  return [
    ...profiles.map((p) => ({ label: p.id === def ? `${p.name} (default)` : p.name, run: () => void createTerminal(p.id) })),
    { separator: true, label: '' },
    { label: 'Choose the default shell…', run: () => setView('settings') },
  ];
}

/** The terminal tab's buttons in the panel header: new, shell picker, clear, kill. */
export function TerminalActions() {
  const active = useApp((s) => s.activeTerminal);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  return (
    <>
      <button className="icon-btn" title="New terminal (Ctrl Shift `)" aria-label="New terminal" onClick={() => void createTerminal()}>
        <Icon name="plus" size={15} />
      </button>
      <button
        className="icon-btn"
        title="New terminal with…"
        aria-label="Choose a shell"
        style={{ width: 20, marginLeft: -6 }}
        onClick={async (e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setMenu({ x: r.left - 180, y: r.bottom + 4, items: await profileMenu() });
        }}
      >
        <Icon name="chevronDown" size={13} />
      </button>
      <button className="icon-btn" title="Clear" aria-label="Clear terminal" disabled={!active} onClick={() => active && clearTerminal(active)}>
        <Icon name="clear" size={15} />
      </button>
      <button className="icon-btn" title="Kill terminal" aria-label="Kill terminal" disabled={!active} onClick={() => active && closeTerminal(active)}>
        <Icon name="trash" size={15} />
      </button>
      {menu && <ContextMenu {...menu} onClose={() => setMenu(null)} />}
    </>
  );
}

export function TerminalView() {
  const terminals = useApp((s) => s.terminals);
  const active = useApp((s) => s.activeTerminal);
  const host = useRef<HTMLDivElement>(null);
  const [profiles, setProfiles] = useState<MenuItem[]>([]);

  useEffect(() => {
    void ensureTerminal();
    void profileMenu().then((items) => setProfiles(items.filter((i) => !i.separator && i.label !== 'Choose the default shell…')));
  }, []);

  useEffect(() => {
    const el = host.current;
    if (!el || !active) return;
    attachTerminal(active, el);
    focusTerminal(active);
    return () => parkTerminals(el);
  }, [active]);

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    let raf = 0;
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        const a = useApp.getState().activeTerminal;
        if (a) fitTerminal(a);
      });
    });
    ro.observe(el);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
    };
  }, []);

  const names = labels(terminals);
  return (
    <div className="terminal-area">
      <div className="terminal-host" ref={host} onMouseDown={() => active && setTimeout(() => focusTerminal(active), 0)} />
      {terminals.length === 0 && (
        <div className="terminal-empty">
          <span className="muted">No terminal is open.</span>
          <div className="row" style={{ flexWrap: 'wrap', justifyContent: 'center' }}>
            {profiles.map((p) => (
              <button key={p.label} className="btn sm" onClick={p.run}>
                <Icon name="terminal" size={14} />
                {p.label}
              </button>
            ))}
          </div>
        </div>
      )}
      {terminals.length > 1 && (
        <div className="terminal-list" role="tablist" aria-label="Terminals" aria-orientation="vertical">
          {terminals.map((t, i) => (
            <div key={t.id} role="tab" tabIndex={0} aria-selected={t.id === active} className="terminal-item" onClick={() => useApp.setState({ activeTerminal: t.id })} onKeyDown={(e) => e.key === 'Enter' && useApp.setState({ activeTerminal: t.id })} title={names[i]}>
              <Icon name="terminal" size={14} />
              <span className="label">{names[i]}</span>
              {t.exited && <span className="faint">exited</span>}
              <button
                className="icon-btn"
                aria-label={`Kill ${names[i]}`}
                title="Kill terminal"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTerminal(t.id);
                }}
              >
                <Icon name="trash" size={13} />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
