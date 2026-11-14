import { signal } from "@preact/signals";
import { useEffect } from "preact/hooks";
import { MapView } from "./map/MapView";
import { level, loadError, manifest, notice, prefs, snapshot } from "./store";
import { Layers, SettingsDialog, SitePanel } from "./ui/Panels";
import { Rail } from "./ui/Rail";
import { closeMenus, TopBar } from "./ui/TopBar";

const cursor = signal<[number, number] | null>(null);

function gridRef(world: [number, number]): string {
  const l = level.value;
  if (!l?.grid_step) return "";
  const column = Math.floor((world[0] - l.bounds[0]) / l.grid_step[0]);
  const row = Math.floor((l.bounds[3] - world[1]) / l.grid_step[1]);
  if (row < 0 || row > 25 || column < 0) return "";
  return `${String.fromCharCode(65 + row)}${column + 1}`;
}

/** A short, quiet tone when a column reaches an enemy site. */
function useAlertTone() {
  useEffect(() => {
    const known = new Set<string>();
    let audio: AudioContext | null = null;
    return snapshot.subscribe((s) => {
      if (!s) return;
      const now = s.alerts.filter((a) => a.phase === "now" && a.kind === "stack").map((a) => a.id);
      const fresh = now.some((id) => !known.has(id));
      known.clear();
      now.forEach((id) => known.add(id));
      if (!fresh || !prefs.value.sound) return;
      try {
        audio ??= new AudioContext();
        const osc = audio.createOscillator(), gain = audio.createGain();
        osc.type = "sine";
        osc.frequency.setValueAtTime(660, audio.currentTime);
        osc.frequency.exponentialRampToValueAtTime(880, audio.currentTime + 0.18);
        gain.gain.setValueAtTime(0.0001, audio.currentTime);
        gain.gain.exponentialRampToValueAtTime(0.06, audio.currentTime + 0.04);
        gain.gain.exponentialRampToValueAtTime(0.0001, audio.currentTime + 0.5);
        osc.connect(gain).connect(audio.destination);
        osc.start();
        osc.stop(audio.currentTime + 0.55);
      } catch {
        /* no audio device: the glow on the map is enough */
      }
    });
  }, []);
}

export function App() {
  useAlertTone();
  const position = cursor.value;
  const player = snapshot.value?.player;
  return (
    <div class={`app ${prefs.value.rail ? "" : "no-rail"}`} onPointerDownCapture={(e) => !(e.target as HTMLElement).closest(".chip-wrap") && closeMenus()}>
      <TopBar />
      <main>
        <MapView onCursor={(world) => (cursor.value = world)} />
        <Layers />
        <SitePanel />
        <div class="readout mono">
          {position ? (
            <>
              {gridRef(position)} <span class="dim">x</span> {Math.round(position[0])} <span class="dim">z</span> {Math.round(position[1])}
              {player && <> <span class="dim">from you</span> {(Math.hypot(position[0] - player.p[0], position[1] - player.p[1]) / 1000).toFixed(1)} km</>}
            </>
          ) : (
            <span class="dim">Drag to move · scroll to zoom · Shift-drag to measure</span>
          )}
        </div>
        {loadError.value && <div class="error">{loadError.value}</div>}
        {!loadError.value && (notice.value || (manifest.value?.pending_game_changes?.length ?? 0) > 0) && (
          <div class="notice">
            {notice.value ?? "The game's event files changed after this map data was built. Positions may be out of date until the next data update."}
          </div>
        )}
      </main>
      <Rail />
      <SettingsDialog />
    </div>
  );
}
