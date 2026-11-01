import { useEffect, useMemo, useState } from 'react';
import { api } from '../api';
import { Icon } from '../icons';
import { useApp, guard, applyChangeset, showPreview, runPackages, openFile, setView } from '../store';
import { ansiToHtml, basename } from '../util';
import type { PackageList } from '../bindings/PackageList';
import type { InstalledPackage } from '../bindings/InstalledPackage';
import type { CatalogEntry } from '../bindings/CatalogEntry';
import type { SearchHit } from '../bindings/SearchHit';
import type { LatestVersion } from '../bindings/LatestVersion';
import type { PackageDetails } from '../bindings/PackageDetails';
import type { Changeset } from '../bindings/Changeset';

type Filter = 'installed' | 'updates' | 'official' | 'works' | 'npm';

const CATEGORIES = ['Security & auth', 'Queues & jobs', 'Data & ORM', 'Validation', 'Transport & realtime', 'Observability'];

/** One row of the middle list, from whichever source it came from. */
interface Row {
  name: string;
  description: string;
  official: boolean;
  installed?: InstalledPackage;
  update?: LatestVersion;
  catalog?: CatalogEntry;
  hit?: SearchHit;
}

/** Renders `code` spans in labels such as "Add `ScheduleModule.forRoot()` to `AppModule`". */
function CodeText({ text }: { text: string }) {
  return (
    <>
      {text.split('`').map((part, i) =>
        i % 2 ? (
          <span key={i} className="mono" style={{ color: 'var(--text)' }}>
            {part}
          </span>
        ) : (
          part
        ),
      )}
    </>
  );
}

function downloads(n: number | null) {
  if (!n) return null;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M weekly`;
  if (n >= 1e3) return `${Math.round(n / 1e3)}k weekly`;
  return `${n} weekly`;
}

function usedIn(p?: InstalledPackage) {
  const n = p?.usedIn.length ?? 0;
  return n ? `used in ${n} file${n === 1 ? '' : 's'}` : null;
}

/** Where an update goes: the newest release for the project's Nest when latest needs a newer one. */
function updateTarget(u?: LatestVersion) {
  return u?.newestForNest ?? u?.latest ?? null;
}

export function PackagesView() {
  const model = useApp((s) => s.model)!;
  const running = useApp((s) => s.packagesRunning);
  const latest = useApp((s) => s.latestVersions);
  const [list, setList] = useState<PackageList | null>(null);
  const [catalog, setCatalog] = useState<CatalogEntry[]>([]);
  const [filter, setFilter] = useState<Filter>('official');
  const [category, setCategory] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [intent, setIntent] = useState<Intent | null>(null);

  const reload = async () => {
    const [l, c] = await Promise.all([guard(api.pkgList()), guard(api.pkgCatalog())]);
    if (l) setList(l);
    if (c) setCatalog(c);
  };

  const checkUpdates = async () => {
    setChecking(true);
    const v = await guard(api.pkgLatest());
    if (v) useApp.setState({ latestVersions: v });
    setChecking(false);
  };

  useEffect(() => {
    void reload();
    if (!useApp.getState().latestVersions) void checkUpdates();
  }, [model.root]);

  // A finished command changes package.json: refresh what's installed and what's outdated.
  const [wasRunning, setWasRunning] = useState(false);
  useEffect(() => {
    if (running) setWasRunning(true);
    else if (wasRunning) {
      setWasRunning(false);
      void reload();
      if (useApp.getState().latestVersions) void checkUpdates();
    }
  }, [running]);

  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setHits(null);
      setSearchError(null);
      return;
    }
    setFilter('npm');
    setCategory(null);
    const t = setTimeout(() => {
      api.pkgSearch(q).then(
        (h) => {
          setHits(h);
          setSearchError(null);
        },
        (e) => setSearchError(e instanceof Error ? e.message : String(e)),
      );
    }, 300);
    return () => clearTimeout(t);
  }, [query]);

  const installedBy = useMemo(() => new Map((list?.packages ?? []).map((p) => [p.name, p])), [list]);
  const updateBy = useMemo(() => new Map((latest ?? []).filter((v) => v.update).map((v) => [v.name, v])), [latest]);
  const fromCatalog = (c: CatalogEntry): Row => ({ name: c.name, description: c.description, official: c.official, catalog: c, installed: installedBy.get(c.name), update: updateBy.get(c.name) });

  const sections: { title: string; sub: string; rows: Row[] }[] = useMemo(() => {
    const fromInstalled = (p: InstalledPackage): Row => {
      const c = catalog.find((x) => x.name === p.name);
      return { name: p.name, description: c?.description ?? (p.installed ? p.range : `${p.range} · not installed`), official: p.name.startsWith('@nestjs/'), installed: p, update: updateBy.get(p.name), catalog: c };
    };
    const inCategory = (c: CatalogEntry) => !category || c.category === category;
    switch (filter) {
      case 'installed': {
        const pkgs = list?.packages ?? [];
        return [
          { title: 'Dependencies', sub: `· ${pkgs.filter((p) => !p.dev).length}`, rows: pkgs.filter((p) => !p.dev).map(fromInstalled) },
          { title: 'Dev dependencies', sub: `· ${pkgs.filter((p) => p.dev).length}`, rows: pkgs.filter((p) => p.dev).map(fromInstalled) },
        ];
      }
      case 'updates':
        return [{ title: 'Updates', sub: model.nestVersion ? `· versions that fit Nest ${model.nestVersion}` : '', rows: (list?.packages ?? []).filter((p) => updateBy.has(p.name)).map(fromInstalled) }];
      case 'npm':
        return [
          {
            title: 'All of npm',
            sub: query.trim() ? `· “${query.trim()}”` : '',
            rows: (hits ?? []).map((h) => ({ name: h.name, description: [h.description, downloads(h.weeklyDownloads)].filter(Boolean).join(' · '), official: h.name.startsWith('@nestjs/'), hit: h, installed: installedBy.get(h.name), update: updateBy.get(h.name), catalog: catalog.find((c) => c.name === h.name) })),
          },
        ];
      case 'works':
        return [{ title: 'Works with Nest', sub: '· third-party', rows: catalog.filter((c) => !c.official && inCategory(c)).map(fromCatalog) }];
      default:
        return [
          { title: 'Nest official', sub: '· from the @nestjs scope', rows: catalog.filter((c) => c.official && inCategory(c)).map(fromCatalog) },
          ...(category ? [{ title: 'Works with Nest', sub: '· third-party', rows: catalog.filter((c) => !c.official && inCategory(c)).map(fromCatalog) }] : []),
        ];
    }
  }, [filter, category, list, catalog, hits, updateBy, query]);

  const allRows = sections.flatMap((s) => s.rows);
  const selectedRow = allRows.find((r) => r.name === selected) ?? (selected ? { name: selected, description: '', official: selected.startsWith('@nestjs/'), installed: installedBy.get(selected), update: updateBy.get(selected) } : null);
  const updatesCount = (list?.packages ?? []).filter((p) => updateBy.has(p.name)).length;

  const pick = (f: Filter, cat: string | null = null) => {
    setFilter(f);
    setCategory(cat);
    if (f !== 'npm') setQuery('');
  };

  return (
    <main className="main">
      <div className="row" style={{ minHeight: 48, flex: 'none', padding: '8px 16px', borderBottom: '1px solid var(--border-subtle)', background: 'var(--bg-panel)', flexWrap: 'wrap', gap: 10 }}>
        <span style={{ fontSize: 14, fontWeight: 600 }}>Packages</span>
        <label className="pkg-search">
          <Icon name="search" size={14} />
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search npm — e.g. @nestjs/bullmq, helmet, stripe" aria-label="Search packages" spellCheck={false} />
          {query && (
            <button className="icon-btn" style={{ width: 22, height: 22 }} aria-label="Clear search" onClick={() => pick('official')}>
              <Icon name="close" size={12} />
            </button>
          )}
        </label>
        {list && (
          <span className="row muted" style={{ marginLeft: 'auto', gap: 6, fontSize: 12 }} title={list.lockfile ?? 'No lockfile; npm is the default'}>
            Using
            <span className="mono pkg-pill">{list.packageManager}</span>
            {list.lockfile ? 'from lockfile' : '(default)'}
          </span>
        )}
        <button className="btn sm" style={{ height: 30, marginLeft: list ? 0 : 'auto' }} disabled={checking} onClick={() => void checkUpdates()}>
          <Icon name="restart" size={14} />
          {checking ? 'Checking…' : 'Check for updates'}
        </button>
      </div>

      <div className="split-v">
        <aside className="pkg-filters" aria-label="Package filters">
          <span className="caps">This project</span>
          <button aria-current={filter === 'installed'} onClick={() => pick('installed')}>
            Installed<span className="faint">{list?.packages.length ?? ''}</span>
          </button>
          <button aria-current={filter === 'updates'} onClick={() => pick('updates')}>
            Updates
            {checking && !latest ? <span className="faint">…</span> : updatesCount > 0 ? <span className="count">{updatesCount}</span> : <span className="faint">0</span>}
          </button>
          <span className="caps">Discover</span>
          <button aria-current={filter === 'official' && !category} onClick={() => pick('official')}>
            Nest official
          </button>
          <button aria-current={filter === 'works' && !category} onClick={() => pick('works')}>
            Works with Nest
          </button>
          <button aria-current={filter === 'npm'} onClick={() => pick('npm')}>
            All of npm
          </button>
          <span className="caps">Category</span>
          {CATEGORIES.map((c) => (
            <button key={c} className="cat" aria-current={category === c} onClick={() => pick('official', category === c ? null : c)}>
              {c}
            </button>
          ))}
        </aside>

        <section aria-label="Package list" className="pkg-list">
          {list?.needsInstall && (filter === 'installed' || filter === 'updates') && (
            <div className="note" style={{ margin: 12, alignItems: 'center' }}>
              <Icon name="warning" size={16} style={{ color: 'var(--warning)' }} />
              <span style={{ flex: 1 }}>Some packages in package.json aren't installed yet.</span>
              <button className="btn sm" disabled={!!running} onClick={() => void runPackages('install', [])}>
                Run {list.packageManager} install
              </button>
            </div>
          )}
          {sections.map((s) => (
            <div key={s.title}>
              <div className="pkg-section">
                <span style={{ fontWeight: 600, color: 'var(--text)' }}>{s.title}</span>
                <span>{s.sub}</span>
              </div>
              {s.rows.map((r) => (
                <PackageRow
                  key={r.name}
                  row={r}
                  selected={r.name === selected}
                  busy={!!running}
                  onSelect={() => setSelected(r.name)}
                  onIntent={(kind) => {
                    setSelected(r.name);
                    setIntent({ name: r.name, kind, n: Date.now() });
                  }}
                />
              ))}
              {s.rows.length === 0 && <EmptySection filter={filter} query={query} hasHits={hits !== null} error={searchError} checking={checking} />}
            </div>
          ))}
        </section>

        {selectedRow ? (
          <Details key={selectedRow.name} row={selectedRow} list={list} intent={intent} />
        ) : (
          <aside aria-label="Package details" className="pkg-details">
            <div className="empty">
              <Icon name="package" size={24} />
              <span>Pick a package to see its versions, compatibility with Nest {model.nestVersion ?? ''} and how it's wired in.</span>
            </div>
          </aside>
        )}
      </div>
    </main>
  );
}

function EmptySection({ filter, query, hasHits, error, checking }: { filter: Filter; query: string; hasHits: boolean; error: string | null; checking: boolean }) {
  let text = 'Nothing here.';
  if (filter === 'npm') text = error ? `Couldn't search: ${error}` : !query.trim() ? 'Type in the search box to search npm.' : hasHits ? 'No packages match.' : 'Searching…';
  else if (filter === 'updates') text = checking ? 'Checking the registry…' : 'Everything is up to date.';
  return <div className="faint" style={{ padding: '14px 16px', fontSize: 12.5 }}>{text}</div>;
}

function PackageRow({ row, selected, busy, onSelect, onIntent }: { row: Row; selected: boolean; busy: boolean; onSelect: () => void; onIntent: (kind: Intent['kind']) => void }) {
  const p = row.installed;
  const sub = [row.description, usedIn(p)].filter(Boolean).join(' · ');
  const target = updateTarget(row.update);
  return (
    <div className={`pk ${selected ? 'selected' : ''}`} onClick={onSelect}>
      <span className={`pk-avatar mono ${row.official ? 'official' : ''}`}>{row.official ? 'N' : '3P'}</span>
      <button className="pk-main" onClick={onSelect} aria-pressed={selected}>
        <span className="row" style={{ gap: 8 }}>
          <span className="mono" style={{ fontSize: 12.5 }}>{row.name}</span>
          {p?.dev && <span className="badge tag">dev</span>}
        </span>
        <span className="pk-sub">{sub}</span>
      </button>
      {p ? (
        <span className="pk-actions">
          {target && (
            <span className="pk-version mono">
              {p.installed ?? p.range} → <span style={{ color: row.update?.update === 'major' ? 'var(--warning)' : 'var(--success)' }}>{target}</span>
            </span>
          )}
          <span className="pk-installed">
            <Icon name="check" size={14} />
            {target ? null : p.installed ? p.installed : 'Not installed'}
          </span>
          <span className="pk-hover">
            {target && (
              <button
                className="btn sm warn"
                disabled={busy}
                onClick={(e) => {
                  e.stopPropagation();
                  void runPackages('add', [`${p.name}@${target}`], p.dev);
                }}
              >
                Update
              </button>
            )}
            <button
              className="btn sm"
              disabled={busy}
              onClick={(e) => {
                e.stopPropagation();
                onIntent('uninstall');
              }}
            >
              Uninstall
            </button>
          </span>
        </span>
      ) : (
        <button
          className={`btn sm ${selected ? 'primary' : ''}`}
          style={{ marginLeft: 'auto' }}
          disabled={busy}
          onClick={(e) => {
            e.stopPropagation();
            onIntent('install');
          }}
        >
          Install
        </button>
      )}
    </div>
  );
}

// ---- details -----------------------------------------------------------------------------------

/** A row button asking the details panel to act: install it, or open the uninstall plan. */
export interface Intent {
  name: string;
  kind: 'install' | 'uninstall';
  n: number;
}

function Details({ row, list, intent }: { row: Row; list: PackageList | null; intent: Intent | null }) {
  const running = useApp((s) => s.packagesRunning);
  const log = useApp((s) => s.logs.packages);
  const [d, setD] = useState<PackageDetails | null>(null);
  const [version, setVersion] = useState('recommended');
  const installed = list?.packages.find((p) => p.name === row.name) ?? row.installed;
  const [dev, setDev] = useState(installed?.dev ?? false);
  const [extras, setExtras] = useState<Record<string, boolean>>({});
  const [wire, setWire] = useState<Record<string, boolean>>({});
  const [uninstalling, setUninstalling] = useState(false);
  const [unwire, setUnwire] = useState<Changeset | null>(null);
  const [removeWiring, setRemoveWiring] = useState(true);
  const [commands, setCommands] = useState<string[]>([]);
  const name = row.name;

  useEffect(() => {
    let live = true;
    setD(null);
    api.pkgDetails(name).then(
      (det) => {
        if (!live) return;
        setD(det);
        setWire(Object.fromEntries(det.wiring.map((w) => [w.id, w.id === 'module' && !w.done])));
      },
      () => {},
    );
    return () => {
      live = false;
    };
  }, [name, installed?.installed, installed?.range]);

  useEffect(() => {
    if (installed) setDev(installed.dev);
    setVersion('recommended');
    setUninstalling(false);
  }, [installed?.installed, installed?.dev]);

  useEffect(() => {
    if (!uninstalling) return;
    void api.pkgUnwire(name).then(setUnwire, () => setUnwire(null));
  }, [uninstalling, name]);

  // Companions, missing peers and @types; each can be turned off.
  const extraList = useMemo(() => {
    if (!d) return [];
    const have = (n: string) => !!list?.packages.some((p) => p.name === n);
    const out: { name: string; dev: boolean; why: string }[] = [];
    const add = (n: string, isDev: boolean, why: string) => !have(n) && n !== name && !out.some((x) => x.name === n) && out.push({ name: n, dev: isDev, why });
    d.catalog?.companions.forEach((c) => add(c, false, 'Used with it'));
    d.peers.forEach((p) => add(p.spec, false, p.note ?? 'Required peer'));
    d.catalog?.devCompanions.forEach((c) => add(c, true, 'Types'));
    if (d.typesPackage) add(d.typesPackage, true, 'TypeScript types');
    return out;
  }, [d, list, name]);
  const chosenExtras = extraList.filter((x) => extras[x.name] ?? true);

  const updateTo = updateTarget(row.update);
  const changedVersion = !!installed && version !== 'recommended' && version !== installed.installed;
  const wantsUpdate = !!installed && version === 'recommended' && !!updateTo;
  const moved = !!installed && dev !== installed.dev;
  const mainSpec = (() => {
    if (installed) {
      if (changedVersion) return `${name}@${version}`;
      if (wantsUpdate) return `${name}@${updateTo}`;
      return moved ? `${name}@${installed.installed ?? installed.range}` : null;
    }
    if (!d || version === 'recommended') return d?.recommended && d.recommended !== d.latest ? `${name}@^${d.recommended}` : name;
    return `${name}@${version}`;
  })();
  const prod = [...(mainSpec ? [mainSpec] : []), ...chosenExtras.filter((x) => !x.dev).map((x) => x.name)];
  const devs = chosenExtras.filter((x) => x.dev).map((x) => x.name);
  const plan = dev ? { prod: [...prod, ...devs], devs: [] as string[] } : { prod, devs };
  const needsRun = plan.prod.length + plan.devs.length > 0;
  const wireSteps = (d?.wiring ?? []).filter((w) => !w.done && wire[w.id]);
  const edited = uninstalling ? (removeWiring && unwire ? unwire.files.map((f) => f.path) : []) : wireSteps.flatMap((w) => w.files);
  const editedFiles = [...new Set([...(needsRun || uninstalling ? ['package.json'] : []), ...edited])];
  const planKey = JSON.stringify([plan, uninstalling]);

  useEffect(() => {
    if (uninstalling) void api.pkgCommand('remove', [name]).then(setCommands, () => {});
    else if (needsRun) void api.pkgCommand('add', plan.prod, dev, plan.devs).then(setCommands, () => {});
    else setCommands([]);
  }, [planKey]);

  const install = async (preview: boolean) => {
    const run = () => {
      if (needsRun) void runPackages('add', plan.prod, dev, plan.devs);
    };
    if (!wireSteps.length) return run();
    const cs = await guard(api.pkgWire(name, wireSteps.map((w) => w.id)));
    if (!cs) return;
    if (preview) showPreview(cs, { title: needsRun ? `Install and wire ${name}` : `Wire ${name}`, onApplied: run });
    else if (await applyChangeset(cs)) run();
  };

  const uninstall = async (preview: boolean) => {
    const run = () => void runPackages('remove', [name]);
    if (!removeWiring || !unwire) return run();
    if (preview) showPreview(unwire, { title: `Uninstall ${name}`, onApplied: run });
    else if (await applyChangeset(unwire)) run();
  };

  // Row buttons: Install runs the default plan once details are in; Uninstall opens the plan.
  const [handled, setHandled] = useState(0);
  useEffect(() => {
    if (!intent || intent.name !== name || intent.n === handled) return;
    if (intent.kind === 'uninstall') {
      setHandled(intent.n);
      setUninstalling(true);
    } else if (d) {
      setHandled(intent.n);
      void install(false);
    }
  }, [intent, d]);

  let primary: string;
  if (!installed) primary = wireSteps.length ? 'Install & wire' : 'Install';
  else if (!needsRun) primary = wireSteps.length ? 'Wire' : 'Up to date';
  else {
    primary = changedVersion ? `Install ${version}` : wantsUpdate ? `Update to ${updateTo}` : moved ? `Move to ${dev ? 'dev' : 'dependencies'}` : 'Install extras';
    if (wireSteps.length) primary += ' & wire';
  }
  const versionOptions = d ? [...new Set([...(installed?.installed ? [installed.installed] : []), ...d.versions])] : [];

  return (
    <aside aria-label="Package details" className="pkg-details">
      <div className="col" style={{ gap: 8 }}>
        <div className="row" style={{ gap: 10, flexWrap: 'wrap' }}>
          <span className="mono selectable" style={{ fontSize: 18, fontWeight: 500, wordBreak: 'break-all' }}>
            {name}
          </span>
          <span className={`badge ${row.official ? 'official' : 'tag'}`}>{row.official ? 'Nest official' : 'Third-party'}</span>
          {installed && <span className="badge ok">{installed.installed ? `Installed ${installed.installed}` : 'Not installed yet'}</span>}
        </div>
        <span className="muted" style={{ lineHeight: '19px' }}>{d?.description ?? row.description}</span>
        <div className="row pkg-links">
          {d?.catalog && (
            <a href={d.catalog.docs} target="_blank" rel="noreferrer">
              Docs
            </a>
          )}
          <a href={d?.npm ?? `https://www.npmjs.com/package/${name}`} target="_blank" rel="noreferrer">
            npm
          </a>
          {d?.repository && (
            <a href={d.repository} target="_blank" rel="noreferrer">
              Repository
            </a>
          )}
          {!d?.repository && d?.homepage && (
            <a href={d.homepage} target="_blank" rel="noreferrer">
              Homepage
            </a>
          )}
          {d?.license && <span>{d.license}</span>}
        </div>
      </div>

      {!d && <span className="faint">Loading versions and compatibility…</span>}
      {d?.registryError && (
        <div className="note">
          <Icon name="warning" size={16} style={{ color: 'var(--warning)', marginTop: 1 }} />
          <span>{d.registryError}. The package manager will pick the version.</span>
        </div>
      )}

      {uninstalling && installed ? (
        <UninstallPlan pkg={installed} unwire={unwire} removeWiring={removeWiring} setRemoveWiring={setRemoveWiring} />
      ) : (
        <>
          <div className="pkg-grid">
            <label className="field">
              <span>Version</span>
              <select className="input mono" value={version} onChange={(e) => setVersion(e.target.value)} disabled={!d || !!d.registryError}>
                <option value="recommended">
                  {installed
                    ? updateTo
                      ? `${updateTo} · update`
                      : `${installed.installed ?? installed.range} · current`
                    : d?.recommended && d.recommended !== d.latest
                      ? `${d.recommended} · newest for Nest ${list?.nestVersion}`
                      : `latest${d?.latest ? ` · ${d.latest}` : ''}`}
                </option>
                {versionOptions.map((v) => (
                  <option key={v} value={v}>
                    {v}
                    {v === installed?.installed ? ' · installed' : v === d?.latest ? ' · latest' : ''}
                  </option>
                ))}
              </select>
            </label>
            <fieldset className="field">
              <legend className="legend" style={{ marginBottom: 6 }}>
                Save as
              </legend>
              <div className="segmented pkg-seg">
                <button aria-pressed={!dev} onClick={() => setDev(false)}>
                  dependency
                </button>
                <button aria-pressed={dev} onClick={() => setDev(true)}>
                  dev
                </button>
              </div>
            </fieldset>
          </div>

          {d && d.checks.length > 0 && (
            <div className="pkg-card">
              <span className="caps">Compatibility</span>
              {d.checks.map((c) => (
                <span key={c.text} className="row" style={{ gap: 8, alignItems: 'flex-start', lineHeight: '18px' }}>
                  <Icon name={c.level === 'ok' ? 'check' : c.level === 'warn' ? 'warning' : 'error'} size={15} style={{ color: c.level === 'ok' ? 'var(--success)' : c.level === 'warn' ? 'var(--warning)' : 'var(--error)', marginTop: 1, flex: 'none' }} />
                  {c.text}
                </span>
              ))}
            </div>
          )}

          {extraList.length > 0 && (
            <div className="col" style={{ gap: 10 }}>
              <span className="caps">Also install</span>
              {extraList.map((x) => (
                <label key={x.name} className="check" style={{ alignItems: 'flex-start' }}>
                  <input type="checkbox" checked={extras[x.name] ?? true} onChange={(e) => setExtras({ ...extras, [x.name]: e.target.checked })} style={{ marginTop: 2 }} />
                  <span>
                    <span className="mono">{x.name}</span>
                    <span className="faint">
                      {' '}
                      · {x.why}
                      {x.dev && !dev ? ' · dev' : ''}
                    </span>
                  </span>
                </label>
              ))}
            </div>
          )}

          {d && (d.wiring.length > 0 || d.catalog?.hint) && (
            <div className="col" style={{ gap: 10 }}>
              <span className="caps">Wire it up</span>
              {d.wiring.map((w) => (
                <label key={w.id} className="check" style={{ alignItems: 'flex-start', lineHeight: '19px' }}>
                  <input type="checkbox" checked={w.done || !!wire[w.id]} disabled={w.done} onChange={(e) => setWire({ ...wire, [w.id]: e.target.checked })} style={{ marginTop: 2 }} />
                  <span className={w.done ? 'faint' : undefined}>
                    <CodeText text={w.label} />
                    {w.done && ' · already done'}
                  </span>
                </label>
              ))}
              {d.catalog?.hint && (
                <div className="note">
                  <Icon name="info" size={16} style={{ color: 'var(--info)', marginTop: 1, flex: 'none' }} />
                  <span style={{ flex: 1 }}>{d.catalog.hint}</span>
                  {/Database view/.test(d.catalog.hint) && (
                    <button className="btn sm" onClick={() => setView('database')}>
                      Open
                    </button>
                  )}
                </div>
              )}
            </div>
          )}
        </>
      )}

      <div className="col" style={{ gap: 8 }}>
        <span className="caps">{running ? 'Running' : 'Will run'}</span>
        {running ? (
          <div className="cmd mono pkg-log">
            {(log ?? []).slice(-8).map((l, i) => (
              <div key={i} dangerouslySetInnerHTML={{ __html: ansiToHtml(l.text) || '&nbsp;' }} />
            ))}
            {!log?.length && <span className="faint">{running}</span>}
          </div>
        ) : (
          <div className="cmd mono" style={{ fontSize: 12, lineHeight: '20px' }}>
            {commands.map((c) => (
              <div key={c}>
                <span className="faint">$</span> {c}
              </div>
            ))}
            {editedFiles.length > 0 ? (
              <div>
                <span className="faint">{commands.length ? 'then edit' : 'edit'}</span> {editedFiles.join(', ')}
              </div>
            ) : (
              !commands.length && <span className="faint">Nothing to do</span>
            )}
          </div>
        )}
      </div>

      <div className="col" style={{ marginTop: 'auto', gap: 10 }}>
        {!uninstalling && !installed && d?.wiring.length ? (
          <div className="note">
            <Icon name="warning" size={16} style={{ color: 'var(--warning)', marginTop: 1, flex: 'none' }} />
            <span>Uninstalling works the same way in reverse: NestFactory lists the files that import the package, then removes the module wiring with the package.</span>
          </div>
        ) : null}
        <div className="row" style={{ gap: 8 }}>
          {uninstalling ? (
            <>
              <button className="btn lg" style={{ flex: 1 }} onClick={() => setUninstalling(false)}>
                Cancel
              </button>
              {unwire && removeWiring && (
                <button className="btn lg" style={{ flex: 1 }} disabled={!!running} onClick={() => void uninstall(true)}>
                  Preview diff
                </button>
              )}
              <button className="btn lg primary" style={{ flex: 1 }} disabled={!!running} onClick={() => void uninstall(false)}>
                Uninstall
              </button>
            </>
          ) : (
            <>
              {installed && (
                <button className="btn lg" style={{ flex: 1 }} disabled={!!running} onClick={() => setUninstalling(true)}>
                  Uninstall
                </button>
              )}
              {(!installed || wireSteps.length > 0) && (
                <button className="btn lg" style={{ flex: 1 }} disabled={!wireSteps.length || !!running} onClick={() => void install(true)}>
                  Preview diff
                </button>
              )}
              <button className="btn lg primary" style={{ flex: 1 }} disabled={!!running || (!needsRun && !wireSteps.length)} onClick={() => void install(false)}>
                {primary}
              </button>
            </>
          )}
        </div>
      </div>
    </aside>
  );
}
function UninstallPlan({ pkg, unwire, removeWiring, setRemoveWiring }: { pkg: InstalledPackage; unwire: Changeset | null; removeWiring: boolean; setRemoveWiring: (v: boolean) => void }) {
  return (
    <div className="col" style={{ gap: 12 }}>
      <span className="caps">Uninstall</span>
      {pkg.essential && (
        <div className="note" style={{ borderColor: 'var(--error-bg)' }}>
          <Icon name="error" size={16} style={{ color: 'var(--error)', marginTop: 1 }} />
          <span>{pkg.name} is needed by every Nest app; removing it breaks the build.</span>
        </div>
      )}
      {pkg.usedIn.length > 0 ? (
        <div className="pkg-card">
          <span className="muted">
            Imported in {pkg.usedIn.length} file{pkg.usedIn.length === 1 ? '' : 's'}. Those imports stop compiling once it's gone:
          </span>
          {pkg.usedIn.map((f) => (
            <button key={f} className="pkg-file mono" onClick={() => openFile(f)}>
              {f}
            </button>
          ))}
        </div>
      ) : (
        <span className="muted">No source file imports it.</span>
      )}
      {unwire && (
        <label className="check" style={{ alignItems: 'flex-start', lineHeight: '19px' }}>
          <input type="checkbox" checked={removeWiring} onChange={(e) => setRemoveWiring(e.target.checked)} style={{ marginTop: 2 }} />
          <span>
            Remove its module wiring from <span className="mono">{unwire.files.map((f) => basename(f.path)).join(', ')}</span>
          </span>
        </label>
      )}
    </div>
  );
}
