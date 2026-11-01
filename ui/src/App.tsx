import { useEffect, useState } from 'react';
import { api, on, launchProject } from './api';
import { useApp, appendLog, openProject, openFile, toast } from './store';
import { applyTheme, reloadFromDisk } from './monaco';
import { TitleBar, ToolRail, StatusBar, Toasts, CommandPalette, useGlobalKeys } from './components/Shell';
import { Workspace } from './components/Workspace';
import { PreviewDialog } from './components/Changes';
import { GeneratorView } from './views/Generator';
import { GraphView, useGraphToast } from './views/Graph';
import { DatabaseView, EntityDesigner } from './views/Database';
import { ApiView } from './views/Api';
import { TestsView } from './views/Tests';
import { EnvView } from './views/Env';
import { SettingsView } from './views/Settings';
import { Welcome, Wizard } from './views/Welcome';
import type { ProjectModel } from './bindings/ProjectModel';
import type { DebugFrame } from './store';

const NOT_TASKS = new Set(['run', 'debug', 'tests']);

function useBackendEvents() {
  useEffect(() => {
    const offs = [
      on('project-updated', (m: ProjectModel) => useApp.setState({ model: m })),
      on('process-output', (p: { id: string; stream: string; text: string }) => {
        appendLog(p.id, { stream: p.stream, text: p.text });
        if (!NOT_TASKS.has(p.id)) appendLog('tasks', { stream: p.stream, text: p.text });
        if ((p.id === 'run' || p.id === 'debug') && /Nest application successfully started/.test(p.text)) useApp.setState({ appStarted: true });
      }),
      on('process-start', (p: { id: string }) => useApp.setState((s) => ({ running: { ...s.running, [p.id]: true } }))),
      on('process-exit', (p: { id: string; code: number | null }) => {
        useApp.setState((s) => ({ running: { ...s.running, [p.id]: false }, appStarted: p.id === 'run' || p.id === 'debug' ? false : s.appStarted }));
        if (p.id === 'run' && p.code && p.code !== 0) toast(`The app exited with code ${p.code}. See the Run panel.`, 'error');
      }),
      on('files-changed', (p: { paths: string[] }) => void reloadFromDisk(p.paths)),
      on('task-done', (p: { id: string; ok: boolean; error: string | null }) => {
        if (p.id === 'install') toast(p.ok ? 'Packages installed' : `Install failed: ${p.error}`, p.ok ? 'success' : 'error');
      }),
      on('debug-state', (p: { state: string; detail: string }) =>
        useApp.setState((s) => ({ debug: { ...s.debug, state: p.state === 'ended' ? 'idle' : p.state, detail: p.detail, frames: p.state === 'paused' ? s.debug.frames : p.state === 'running' ? null : s.debug.frames } })),
      ),
      on('debug-paused', (p: { frames: DebugFrame[] }) => {
        useApp.setState((s) => ({ debug: { ...s.debug, state: 'paused', frames: p.frames } }));
        const top = p.frames.find((f) => !f.external && f.file);
        if (top?.file) openFile(top.file, top.line);
      }),
      on('debug-resumed', () => useApp.setState((s) => ({ debug: { ...s.debug, frames: null } }))),
    ];
    return () => offs.forEach((off) => off());
  }, []);
}

function ProjectBody() {
  const view = useApp((s) => s.view);
  return (
    <div className="body">
      <ToolRail />
      {view === 'explorer' && <Workspace />}
      {view === 'graph' && <GraphView />}
      {view === 'generate' && <GeneratorView />}
      {view === 'database' && <DatabaseView />}
      {view === 'api' && <ApiView />}
      {view === 'tests' && <TestsView />}
      {view === 'env' && <EnvView />}
      {view === 'settings' && <SettingsView />}
    </div>
  );
}

export function App() {
  const screen = useApp((s) => s.screen);
  const model = useApp((s) => s.model);
  const [dbLabel, setDbLabel] = useState<string | null>(null);
  useBackendEvents();
  useGlobalKeys();
  useGraphToast();

  useEffect(() => {
    (async () => {
      try {
        const settings = await api.getSettings();
        useApp.setState({ settings });
        applyTheme(settings.theme);
      } catch (e) {
        toast(`Can't reach the NestFactory backend: ${e instanceof Error ? e.message : e}`, 'error');
        applyTheme('dark');
        return;
      }
      api.toolchain().then((toolchain) => useApp.setState({ toolchain }), () => {});
      const p = await launchProject().catch(() => null);
      if (p) await openProject(p);
    })();
  }, []);

  useEffect(() => {
    if (!model) {
      setDbLabel(null);
      return;
    }
    api.dbProfiles().then(
      (ps) => {
        const p = ps[0];
        setDbLabel(p ? `${{ postgres: 'PostgreSQL', mysql: 'MySQL', sqlite: 'SQLite', mongodb: 'MongoDB' }[p.config.database]} · ${p.config.databaseName}` : null);
      },
      () => setDbLabel(null),
    );
  }, [model?.root, model?.dependencies.length]);

  return (
    <div className="app">
      <TitleBar />
      {screen === 'welcome' && <Welcome />}
      {screen === 'wizard' && <Wizard />}
      {screen === 'project' && model && <ProjectBody />}
      {screen === 'project' && <StatusBar dbLabel={dbLabel} />}
      <PreviewDialog />
      <EntityDesigner />
      <CommandPalette />
      <Toasts />
    </div>
  );
}
