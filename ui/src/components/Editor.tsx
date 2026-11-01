import { useEffect, useRef, useState } from 'react';
import { monaco, getModel, saveFile, updateMarkers, uriFor } from '../monaco';
import { useApp, closeTab, toggleBreakpoint } from '../store';
import { Icon } from '../icons';
import { basename, kindOf, KIND_LETTER, languageFor } from '../util';

const viewStates = new Map<string, monaco.editor.ICodeEditorViewState | null>();

export function CodeEditor({ path, showCoverage = false }: { path: string | null; showCoverage?: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const editor = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const decorations = useRef<monaco.editor.IEditorDecorationsCollection | null>(null);
  const current = useRef<string | null>(null);
  const fontSize = useApp((s) => s.settings?.editorFontSize ?? 13);
  const breakpoints = useApp((s) => s.breakpoints);
  const frames = useApp((s) => s.debug.frames);
  const testRun = useApp((s) => s.testRun);
  const reveal = useApp((s) => s.reveal);
  const [ready, setReady] = useState(0);

  useEffect(() => {
    const ed = monaco.editor.create(host.current!, {
      model: null,
      fontFamily: "'Geist Mono Variable', 'Geist Mono', Consolas, monospace",
      fontSize,
      lineHeight: Math.round(fontSize * 1.54),
      minimap: { enabled: false },
      glyphMargin: true,
      scrollBeyondLastLine: false,
      automaticLayout: true,
      codeLens: true,
      tabSize: 2,
      padding: { top: 10 },
      renderLineHighlight: 'line',
      fixedOverflowWidgets: true,
      smoothScrolling: true,
      lineNumbersMinChars: 3,
      'semanticHighlighting.enabled': false,
      scrollbar: { verticalScrollbarSize: 10, horizontalScrollbarSize: 10 },
    });
    editor.current = ed;
    decorations.current = ed.createDecorationsCollection();
    ed.onDidChangeCursorPosition((e) => useApp.setState({ cursor: { line: e.position.lineNumber, col: e.position.column } }));
    ed.onMouseDown((e) => {
      if (e.target.type === monaco.editor.MouseTargetType.GUTTER_GLYPH_MARGIN && current.current?.endsWith('.ts') && e.target.position) {
        toggleBreakpoint(current.current, e.target.position.lineNumber);
      }
    });
    ed.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
      if (current.current) void saveFile(current.current);
    });
    ed.addCommand(monaco.KeyCode.F9, () => {
      const pos = ed.getPosition();
      if (pos && current.current?.endsWith('.ts')) toggleBreakpoint(current.current, pos.lineNumber);
    });
    return () => {
      if (current.current) viewStates.set(current.current, ed.saveViewState());
      ed.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    editor.current?.updateOptions({ fontSize, lineHeight: Math.round(fontSize * 1.54) });
  }, [fontSize]);

  useEffect(() => {
    const ed = editor.current;
    if (!ed) return;
    if (current.current) viewStates.set(current.current, ed.saveViewState());
    current.current = path;
    if (!path) {
      ed.setModel(null);
      return;
    }
    let cancelled = false;
    getModel(path).then(
      (m) => {
        if (cancelled || current.current !== path) return;
        ed.setModel(m);
        const vs = viewStates.get(path);
        if (vs) ed.restoreViewState(vs);
        updateMarkers();
        setReady((n) => n + 1);
      },
      (e) => {
        if (cancelled) return;
        ed.setModel(monaco.editor.createModel(`// Could not open ${path}\n// ${e}`, 'plaintext'));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [path]);

  useEffect(() => {
    const ed = editor.current;
    if (!ed || !reveal || reveal.path !== path || !ed.getModel()) return;
    ed.revealLineInCenter(reveal.line);
    ed.setPosition({ lineNumber: reveal.line, column: 1 });
    ed.focus();
  }, [reveal, path, ready]);

  useEffect(() => {
    const ed = editor.current;
    const coll = decorations.current;
    if (!ed || !coll || !path || !ed.getModel()) return;
    const decos: monaco.editor.IModelDeltaDecoration[] = [];
    for (const b of breakpoints.filter((x) => x.file === path)) {
      decos.push({ range: new monaco.Range(b.line, 1, b.line, 1), options: { glyphMarginClassName: 'bp-glyph', glyphMarginHoverMessage: { value: 'Breakpoint (F9)' } } });
    }
    const top = frames?.[0];
    if (top && top.file === path) {
      decos.push({ range: new monaco.Range(top.line, 1, top.line, 1), options: { isWholeLine: true, className: 'debug-line' } });
    }
    if (showCoverage && testRun?.coverage) {
      const fc = testRun.coverage.files.find((f) => f.file === path);
      for (const l of fc?.covered ?? []) decos.push({ range: new monaco.Range(l, 1, l, 1), options: { linesDecorationsClassName: 'cov-hit' } });
      for (const l of fc?.uncovered ?? []) decos.push({ range: new monaco.Range(l, 1, l, 1), options: { linesDecorationsClassName: 'cov-miss' } });
    }
    const suite = testRun?.suites.find((s) => s.file === path);
    for (const t of suite?.tests ?? []) {
      if (!t.line || (t.status !== 'passed' && t.status !== 'failed')) continue;
      decos.push({
        range: new monaco.Range(t.line, 1, t.line, 1),
        options: { after: { content: t.status === 'passed' ? '  ✓ passed' : '  ✕ failed', inlineClassName: t.status === 'passed' ? 'mono faint' : 'mono' }, hoverMessage: t.failure ? { value: '```\n' + t.failure.slice(0, 1500) + '\n```' } : undefined },
      });
    }
    coll.set(decos);
  }, [breakpoints, frames, testRun, path, showCoverage, ready]);

  return <div className="editor-host" ref={host} />;
}

export function DiffView({ path, original, modified, inline }: { path: string; original: string; modified: string; inline: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const diff = useRef<monaco.editor.IStandaloneDiffEditor | null>(null);
  useEffect(() => {
    diff.current = monaco.editor.createDiffEditor(host.current!, {
      readOnly: true,
      automaticLayout: true,
      renderSideBySide: !inline,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      fontFamily: "'Geist Mono Variable', 'Geist Mono', Consolas, monospace",
      fontSize: 12.5,
      lineHeight: 21,
      renderOverviewRuler: false,
      originalEditable: false,
      hideUnchangedRegions: { enabled: false },
      useInlineViewWhenSpaceIsLimited: false,
    });
    return () => {
      const m = diff.current?.getModel();
      diff.current?.dispose();
      m?.original.dispose();
      m?.modified.dispose();
    };
  }, []);
  useEffect(() => {
    diff.current?.updateOptions({ renderSideBySide: !inline });
  }, [inline]);
  useEffect(() => {
    const d = diff.current;
    if (!d) return;
    const old = d.getModel();
    const lang = languageFor(path);
    const original_ = monaco.editor.createModel(original, lang);
    const modified_ = monaco.editor.createModel(modified, lang);
    d.setModel({ original: original_, modified: modified_ });
    old?.original.dispose();
    old?.modified.dispose();
  }, [path, original, modified]);
  return <div style={{ position: 'absolute', inset: 0 }} ref={host} />;
}

function Crumbs({ path }: { path: string }) {
  const model = useApp((s) => s.model);
  const parts = path.split('/');
  const classes = model?.classes.filter((c) => c.file === path) ?? [];
  const primary = classes.find((c) => c.kind !== 'dto' && c.kind !== 'other') ?? classes[0];
  let status: { text: string; tone: 'ok' | 'warn' | 'err' } | null = null;
  if (primary && model) {
    if (primary.kind === 'module') {
      const m = model.modules.find((x) => x.name === primary.name);
      const hasIssue = model.diagnostics.some((d) => d.modules.includes(primary.name) && d.severity !== 'info');
      status = { text: `${m?.providers.length ?? 0} providers · ${m?.imports.length ?? 0} imports`, tone: hasIssue ? 'warn' : 'ok' };
    } else if (primary.modules.length) {
      const missing = primary.injects.some((i) => i.status === 'missing');
      status = missing ? { text: `Missing provider in ${primary.modules[0]}`, tone: 'err' } : { text: `Wired in ${primary.modules[0]}`, tone: 'ok' };
    } else if (['controller', 'service', 'provider', 'gateway', 'resolver'].includes(primary.kind)) {
      status = { text: 'Not registered in a module', tone: 'warn' };
    }
  }
  return (
    <div className="crumbs">
      {parts.map((p, i) => (
        <span key={i} style={{ display: 'contents' }}>
          {i > 0 && <span>/</span>}
          <span style={i === parts.length - 1 ? { color: 'var(--text-2)' } : undefined}>{p}</span>
        </span>
      ))}
      {status && (
        <span style={{ marginLeft: 'auto', display: 'flex', gap: 6, alignItems: 'center', color: status.tone === 'err' ? 'var(--error)' : status.tone === 'warn' ? 'var(--warning)' : undefined }}>
          <span className={`dot ${status.tone}`} />
          {status.text}
        </span>
      )}
    </div>
  );
}

export function EditorTabs() {
  const tabs = useApp((s) => s.tabs);
  const active = useApp((s) => s.active);
  const close = async (path: string, dirty: boolean) => {
    if (dirty && !window.confirm(`${basename(path)} has unsaved changes. Close without saving?`)) return;
    if (dirty) {
      // Drop the unsaved buffer so reopening shows the file on disk.
      monaco.editor.getModel(uriFor(path))?.dispose();
    }
    closeTab(path);
  };
  return (
    <div role="tablist" className="tabs">
      {tabs.map((t) => {
        const k = kindOf(t.path);
        return (
          <button
            key={t.path}
            role="tab"
            className="tab"
            aria-selected={t.path === active}
            title={t.path}
            onClick={() => useApp.setState({ active: t.path })}
            onAuxClick={(e) => e.button === 1 && void close(t.path, t.dirty)}
          >
            <span className={`kind ${k}`} style={{ background: 'none', width: 'auto' }}>
              {KIND_LETTER[k]}
            </span>
            {basename(t.path)}
            {t.dirty ? (
              <span className="x" style={{ visibility: 'visible' }} onClick={(e) => { e.stopPropagation(); void close(t.path, true); }} aria-label="Unsaved; close">
                <span className="dirty" />
              </span>
            ) : (
              <span className="x" onClick={(e) => { e.stopPropagation(); void close(t.path, false); }} aria-label={`Close ${basename(t.path)}`}>
                <Icon name="close" size={12} />
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}

export function EditorArea({ showCoverage = false }: { showCoverage?: boolean }) {
  const active = useApp((s) => s.active);
  return (
    <>
      <EditorTabs />
      {active ? <Crumbs path={active} /> : null}
      {active ? (
        <CodeEditor path={active} showCoverage={showCoverage} />
      ) : (
        <div className="editor-host">
          <div className="empty">
            <Icon name="file" size={28} />
            <span>Open a file from the explorer, or press Ctrl P.</span>
          </div>
        </div>
      )}
    </>
  );
}
