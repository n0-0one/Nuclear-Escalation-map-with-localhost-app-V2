// Talks to the engine. Inside the desktop app this goes through Tauri;
// in a browser (development) it goes through the local dev server.

import type { Snapshot } from "./types";

const tauri = (window as any).__TAURI_INTERNALS__ as { invoke: (cmd: string, args?: unknown) => Promise<any> } | undefined;
export const isDesktop = !!tauri;

const packBase = (() => {
  if (!tauri) return "/pack/";
  // Custom protocols look different on Windows (WebView2) and elsewhere.
  return navigator.userAgent.includes("Windows") ? "http://pack.localhost/" : "pack://localhost/";
})();

export const packUrl = (path: string) => packBase + path;

export async function packJson<T>(path: string): Promise<T> {
  const response = await fetch(packUrl(path));
  if (!response.ok) throw new Error(`${path}: ${response.status}`);
  return response.json();
}

export async function command<T = unknown>(cmd: string, args: unknown = {}): Promise<T> {
  if (tauri) return tauri.invoke("engine", { cmd, args });
  const response = await fetch("/api/cmd", { method: "POST", body: JSON.stringify({ cmd, args }) });
  const body = await response.json();
  if (body.error) throw new Error(body.error);
  return body.ok as T;
}

/** Desktop-only actions (window handling, updates). Resolves to null in a browser. */
export async function shell<T = unknown>(cmd: string, args: Record<string, unknown> = {}): Promise<T | null> {
  if (!tauri) return null;
  return tauri.invoke(cmd, args);
}

export function subscribe(onSnapshot: (snapshot: Snapshot) => void): () => void {
  let stopped = false;
  if (tauri) {
    // Lazy import keeps the Tauri event plumbing out of the browser build path.
    let unlisten: (() => void) | undefined;
    const events = (window as any).__TAURI__?.event;
    events?.listen("snapshot", (event: { payload: string }) => {
      if (!stopped) onSnapshot(JSON.parse(event.payload));
    }).then((stop: () => void) => (unlisten = stop));
    command<Snapshot>("snapshot").then((s) => !stopped && onSnapshot(s)).catch(() => {});
    return () => {
      stopped = true;
      unlisten?.();
    };
  }
  let lastSeq = -1;
  const tick = async () => {
    if (stopped) return;
    try {
      const response = await fetch("/api/snapshot");
      const snapshot: Snapshot = await response.json();
      if (snapshot.seq !== lastSeq) {
        lastSeq = snapshot.seq;
        onSnapshot(snapshot);
      }
    } catch {
      /* dev server not running yet */
    }
    setTimeout(tick, document.hidden ? 1000 : 300);
  };
  tick();
  return () => {
    stopped = true;
  };
}
