import { useEffect, useState } from 'react';
import { api } from '../api';
import { useApp, guard, toast } from '../store';
import { applyTheme } from '../monaco';
import type { Settings } from '../bindings/Settings';
import type { Toolchain } from '../bindings/Toolchain';

export function SettingsView() {
  const settings = useApp((s) => s.settings);
  const [tc, setTc] = useState<Toolchain | null>(useApp.getState().toolchain);
  useEffect(() => {
    api.toolchain().then((t) => {
      setTc(t);
      useApp.setState({ toolchain: t });
    }, () => {});
  }, []);
  if (!settings) return <main className="page" />;
  const save = async (patch: Partial<Settings>) => {
    const next = { ...settings, ...patch };
    useApp.setState({ settings: next });
    if (patch.theme) applyTheme(patch.theme);
    await guard(api.saveSettings(next));
  };
  const tool = (label: string, v: string | null, hint: string) => (
    <div className="row" style={{ fontSize: 12.5 }}>
      <span className={`dot ${v ? 'ok' : 'err'}`} />
      <span style={{ width: 90 }}>{label}</span>
      <span className="mono muted">{v ?? `not found — ${hint}`}</span>
    </div>
  );
  return (
    <main className="page">
      <div className="page-inner" style={{ maxWidth: 720 }}>
        <h1>Settings</h1>
        <section className="form-grid">
          <label className="field">
            <span>Theme</span>
            <select className="input" value={settings.theme} onChange={(e) => void save({ theme: e.target.value })}>
              <option value="dark">Forge Dark</option>
              <option value="light">Forge Light</option>
              <option value="system">Match system</option>
            </select>
          </label>
          <label className="field">
            <span>Editor font size</span>
            <input className="input" type="number" min={10} max={24} value={settings.editorFontSize} onChange={(e) => void save({ editorFontSize: Math.max(10, Math.min(24, Number(e.target.value) || 13)) })} />
          </label>
          <label className="field">
            <span>Default package manager</span>
            <select className="input" value={settings.defaultPackageManager} onChange={(e) => void save({ defaultPackageManager: e.target.value as Settings['defaultPackageManager'] })}>
              {(['npm', 'pnpm', 'yarn', 'bun'] as const).map((p) => (
                <option key={p} value={p} disabled={!!tc && !tc.packageManagers.includes(p)}>
                  {p}
                  {tc && !tc.packageManagers.includes(p) ? ' (not installed)' : ''}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>New projects go in</span>
            <input className="input mono" value={settings.defaultLocation} onChange={(e) => useApp.setState({ settings: { ...settings, defaultLocation: e.target.value } })} onBlur={() => void save({})} />
          </label>
        </section>
        <label className="check">
          <input type="checkbox" checked={settings.formatOnSave} onChange={(e) => void save({ formatOnSave: e.target.checked })} />
          Format on save with the project's Prettier (TypeScript's formatter when it has none)
        </label>
        <label className="check">
          <input type="checkbox" checked={settings.typeCheckOnSave} onChange={(e) => void save({ typeCheckOnSave: e.target.checked })} />
          Type-check with the project's TypeScript after saving a .ts file
        </label>
        <section className="col" style={{ gap: 10 }}>
          <span className="caps">Toolchain</span>
          {tc ? (
            <>
              {tool('Node.js', tc.node, 'install Node.js LTS')}
              {tool('Nest CLI', tc.nest, 'npm i -g @nestjs/cli (npx is used otherwise)')}
              {tool('Docker', tc.docker, 'needed only for local databases')}
              {tool('Git', tc.git, 'optional')}
              <div className="row faint" style={{ fontSize: 12.5 }}>
                Package managers: {tc.packageManagers.join(', ') || 'none found'}
              </div>
            </>
          ) : (
            <span className="faint">Checking…</span>
          )}
        </section>
        <section className="col" style={{ gap: 8 }}>
          <span className="caps">Privacy</span>
          <span className="muted" style={{ lineHeight: '19px' }}>
            NestFactory sends no telemetry. Source code and connection strings never leave this machine; database passwords are kept in the OS keychain. Settings and saved requests live in the app data folder, never in your project.
          </span>
        </section>
        <section className="col" style={{ gap: 8 }}>
          <span className="caps">About</span>
          <span className="muted">NestFactory 0.1.0 · Rust core, Tauri shell, Monaco editor.</span>
          <button className="btn" style={{ alignSelf: 'flex-start' }} onClick={() => toast('Shortcuts: Ctrl K commands · Ctrl P files · Ctrl Alt G generate · Ctrl Z undo generation · F5 debug · F9 breakpoint · Ctrl ` panel')}>
            Keyboard shortcuts
          </button>
        </section>
      </div>
    </main>
  );
}
