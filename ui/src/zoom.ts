// Window zoom, as in VS Code: Ctrl + and Ctrl - step through the levels below and Ctrl 0 resets.
// The desktop app zooms the webview itself, so text stays crisp and the editor and terminal
// keep exact mouse positions; the level is kept between runs. In a browser (dev bridge) the
// browser's own zoom already does this, so the keys are left to it.
import { isTauri } from './api';
import { useApp, toast, errorText } from './store';

const LEVELS = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3];

function saved() {
  try {
    const z = Number(localStorage.getItem('nf.zoom'));
    return LEVELS.includes(z) ? z : 1;
  } catch {
    return 1;
  }
}

export async function setZoom(z: number) {
  if (!isTauri) {
    toast('In the browser, use its own zoom (Ctrl + / Ctrl -).');
    return;
  }
  try {
    const { getCurrentWebview } = await import('@tauri-apps/api/webview');
    await getCurrentWebview().setZoom(z);
  } catch (e) {
    toast(`Couldn't zoom: ${errorText(e)}`, 'error');
    return;
  }
  useApp.setState({ zoom: z });
  try {
    localStorage.setItem('nf.zoom', String(z));
  } catch {
    /* storage unavailable */
  }
}

export function zoomIn() {
  const z = useApp.getState().zoom;
  void setZoom(LEVELS.find((l) => l > z + 0.001) ?? z);
}

export function zoomOut() {
  const z = useApp.getState().zoom;
  void setZoom([...LEVELS].reverse().find((l) => l < z - 0.001) ?? z);
}

export function resetZoom() {
  void setZoom(1);
}

/** Applies the saved level at startup. */
export function restoreZoom() {
  const z = saved();
  if (isTauri && z !== 1) void setZoom(z);
}

/** The zoom key in `e`, if any: Ctrl with + (or =), - and 0, on the main keys or the keypad. */
export function zoomKey(e: KeyboardEvent): (() => void) | null {
  if (!(e.ctrlKey || e.metaKey) || e.altKey) return null;
  if (e.key === '=' || e.key === '+' || e.code === 'NumpadAdd') return zoomIn;
  if (e.key === '-' || e.key === '_' || e.code === 'NumpadSubtract') return zoomOut;
  if ((e.key === '0' && !e.shiftKey) || e.code === 'Numpad0') return resetZoom;
  return null;
}
