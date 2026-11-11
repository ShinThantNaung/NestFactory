// Stroke icons drawn for NestFactory, matching the design artboards.
import type { ReactNode } from 'react';
import logoUrl from './assets/logo.png';

const paths: Record<string, ReactNode> = {
  file: (
    <>
      <path d="M6 3h8l4 4v14H6z" />
      <path d="M14 3v4h4" />
    </>
  ),
  graph: (
    <>
      <circle cx="6" cy="6" r="2.5" />
      <circle cx="18" cy="8" r="2.5" />
      <circle cx="10" cy="18" r="2.5" />
      <path d="M8.4 6.5l7.2 1.2M7 8.3l2.2 7.3M16.5 10.2l-5 5.8" />
    </>
  ),
  generate: (
    <>
      <rect x="4" y="4" width="16" height="16" rx="3" />
      <path d="M12 8v8M8 12h8" />
    </>
  ),
  database: (
    <>
      <ellipse cx="12" cy="6" rx="7" ry="3" />
      <path d="M5 6v12c0 1.7 3.1 3 7 3s7-1.3 7-3V6" />
      <path d="M5 12c0 1.7 3.1 3 7 3s7-1.3 7-3" />
    </>
  ),
  package: (
    <>
      <path d="M3 8l9-5 9 5v8l-9 5-9-5z" />
      <path d="M3 8l9 5 9-5M12 13v8M7.5 5.5l9 5" />
    </>
  ),
  api: <path d="M3 12h4l3-7 4 14 3-7h4" />,
  tests: <path d="M9 3h6M10 3v6l-5 9a2 2 0 0 0 1.7 3h10.6a2 2 0 0 0 1.7-3l-5-9V3" />,
  env: (
    <>
      <path d="M4 7h10M18 7h2M4 17h4M12 17h8" />
      <circle cx="16" cy="7" r="2" />
      <circle cx="10" cy="17" r="2" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M5.6 18.4l2.1-2.1M16.3 7.7l2.1-2.1" />
    </>
  ),
  search: (
    <>
      <circle cx="11" cy="11" r="6" />
      <path d="M20 20l-4.5-4.5" />
    </>
  ),
  chevronDown: <path d="M6 9l6 6 6-6" />,
  chevronRight: <path d="M9 6l6 6-6 6" />,
  restart: (
    <>
      <path d="M4 12a8 8 0 1 0 2.4-5.7" />
      <path d="M4 4v4h4" />
    </>
  ),
  bug: (
    <>
      <rect x="7" y="8" width="10" height="12" rx="5" />
      <path d="M12 11v9M4 13h3M17 13h3M9 5l1 2.5M15 5l-1 2.5" />
    </>
  ),
  minimize: <path d="M5 12h14" />,
  maximize: <rect x="5" y="5" width="14" height="14" />,
  close: <path d="M6 6l12 12M18 6L6 18" />,
  plus: <path d="M12 5v14M5 12h14" />,
  check: <path d="M5 12l4 4 10-10" />,
  warning: (
    <>
      <path d="M12 3l9 16H3z" />
      <path d="M12 10v4M12 17h.01" />
    </>
  ),
  error: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 8v5M12 16h.01" />
    </>
  ),
  info: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v5M12 8h.01" />
    </>
  ),
  module: (
    <>
      <path d="M12 3l8 4.5v9L12 21l-8-4.5v-9z" />
      <path d="M4 7.5l8 4.5 8-4.5M12 12v9" />
    </>
  ),
  docker: (
    <>
      <rect x="3" y="10" width="18" height="9" rx="2" />
      <path d="M6 10V7h3v3M10 10V7h3v3M14 10V7h3v3M10 4h3v3" />
    </>
  ),
  lock: (
    <>
      <rect x="5" y="11" width="14" height="10" rx="2" />
      <path d="M8 11V8a4 4 0 0 1 8 0v3" />
    </>
  ),
  stepOver: (
    <>
      <path d="M4 13a8 8 0 0 1 15-3" />
      <path d="M20 5v5h-5" />
      <circle cx="12" cy="19" r="1.5" />
    </>
  ),
  stepInto: (
    <>
      <path d="M12 3v11M7 9l5 5 5-5" />
      <circle cx="12" cy="19" r="1.5" />
    </>
  ),
  stepOut: (
    <>
      <path d="M12 14V3M7 8l5-5 5 5" />
      <circle cx="12" cy="19" r="1.5" />
    </>
  ),
  folder: <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />,
  fileAdd: (
    <>
      <path d="M12 21H6V3h8l4 4v5" />
      <path d="M14 3v4h4M18 15v6M15 18h6" />
    </>
  ),
  folderAdd: (
    <>
      <path d="M12 19H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v3" />
      <path d="M18 14v6M15 17h6" />
    </>
  ),
  collapseAll: (
    <>
      <path d="M8 4h10a2 2 0 0 1 2 2v10" />
      <rect x="4" y="8" width="12" height="12" rx="2" />
      <path d="M7.5 14h5" />
    </>
  ),
  terminal: (
    <>
      <path d="M5 8l4 4-4 4M12 16h7" />
    </>
  ),
  clear: <path d="M4 7h12M4 12h9M4 17h6M15 15l5 5M20 15l-5 5" />,
  send: <path d="M4 12l16-8-6 16-2-7z" />,
  trash: <path d="M5 7h14M10 7V5h4v2M7 7l1 13h8l1-13" />,
  save: (
    <>
      <path d="M5 4h11l3 3v13H5z" />
      <path d="M8 4v5h7V4M8 20v-6h8v6" />
    </>
  ),
  table: (
    <>
      <rect x="4" y="5" width="16" height="14" rx="2" />
      <path d="M4 10h16M10 10v9" />
    </>
  ),
  external: <path d="M14 4h6v6M20 4l-9 9M18 14v6H4V6h6" />,
  undo: (
    <>
      <path d="M9 14L4 9l5-5" />
      <path d="M4 9h10a6 6 0 0 1 0 12h-3" />
    </>
  ),
};

export function Icon({ name, size, className, style }: { name: keyof typeof paths | string; size?: number; className?: string; style?: React.CSSProperties }) {
  const s = size ? { width: size, height: size, ...style } : style;
  return (
    <svg className={`ico ${className ?? ''}`} viewBox="0 0 24 24" style={s} aria-hidden="true">
      {paths[name] ?? null}
    </svg>
  );
}

/** A window outline with one part (left bar, bottom panel, right bar) drawn solid when it's showing. */
export function LayoutIcon({ part, on, size = 16 }: { part: 'left' | 'bottom' | 'right'; on: boolean; size?: number }) {
  const region = part === 'left' ? { x: 3, y: 4, w: 6, h: 16 } : part === 'right' ? { x: 15, y: 4, w: 6, h: 16 } : { x: 3, y: 13, w: 18, h: 7 };
  const divider = part === 'left' ? 'M9 4v16' : part === 'right' ? 'M15 4v16' : 'M3 13h18';
  return (
    <svg className="ico" viewBox="0 0 24 24" width={size} height={size} aria-hidden="true">
      {on && <rect x={region.x} y={region.y} width={region.w} height={region.h} style={{ fill: 'currentColor', stroke: 'none', opacity: 0.55 }} />}
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d={divider} />
    </svg>
  );
}

export function Play({ size = 12 }: { size?: number }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true">
      <path d="M7 5l12 7-12 7z" fill="currentColor" />
    </svg>
  );
}

export function StopSquare({ size = 12 }: { size?: number }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true">
      <rect x="5" y="5" width="14" height="14" rx="2" fill="currentColor" />
    </svg>
  );
}

export function Logo({ size = 20 }: { size?: number }) {
  return <img src={logoUrl} width={size} height={size} alt="" aria-hidden="true" draggable={false} style={{ display: 'block', flex: 'none' }} />;
}
