// Site details, layer switches and the settings dialog.

import { signal } from "@preact/signals";
import { command, isDesktop, packUrl, shell } from "../transport";
import { applySettings, core, enemyTeam, eraIndex, isLive, manifest, prefs, selected, setPref, siteName, siteStates, snapshot, toggleLayer, variant, view } from "../store";
import type { Placement, Proof, Status } from "../types";
import { clock, km, weaponLabel } from "./format";
import { settingsOpen } from "./TopBar";

const STATE_TEXT: Record<string, string> = {
  "alive/confirmed": "On the map",
  "alive/manual": "Alive (marked by you)",
  "destroyed/confirmed": "Destroyed (kill feed)",
  "destroyed/likely": "Probably destroyed",
  "destroyed/manual": "Destroyed (marked by you)",
};

function stateText(status: Status, proof: Proof, visible: boolean) {
  if (status === "alive" && !visible && proof !== "manual") return "Seen earlier";
  return STATE_TEXT[`${status}/${proof}`] ?? "Not seen yet";
}

export function SitePanel() {
  const index = selected.value, v = variant.value, c = core.value;
  if (index === null || !v || !c) return null;
  const site = v.sites[index];
  if (!site) return null;
  const bit = eraIndex.value;
  const state = siteStates.value.get(index);
  const live = isLive.value;
  const enemy = site.team === enemyTeam.value;
  const units = site.units.map((u, slot) => [u, slot] as [Placement, number]).filter(([u]) => (u.e >> bit) & 1);
  const reach = Math.max(0, ...units.map(([u]) => c.units[u.u]?.eng ?? c.units[u.u]?.ai_eng ?? u.tr ?? 0));
  const radar = Math.max(0, ...units.map(([u]) => c.units[u.u]?.det ?? 0));
  const first = units[0] ? c.units[units[0][0].u] : undefined;
  const art = first && c.unit_art.includes(first.model) ? packUrl(`units/${first.model}.webp`) : null;
  const mark = (slot: number | null, destroyed: boolean | null) =>
    command("mark", { site: index, ...(slot !== null ? { slot } : {}), ...(destroyed !== null ? { destroyed } : {}) });
  const schedule = snapshot.value?.schedules.find((s) => s.site === index);

  return (
    <section class="site-panel">
      <header>
        <div>
          <span class={`side-tag ${enemy ? "hostile" : "friendly"}`}>{enemy ? "Enemy" : "Friendly"} · Team {site.team}</span>
          <h2>{siteName(site)}</h2>
        </div>
        <button class="close" onClick={() => (selected.value = null)} aria-label="Close">×</button>
      </header>
      {art && <img class="art" src={art} alt="" loading="lazy" />}
      <dl class="facts">
        {reach > 0 && (<><dt>Weapon reach</dt><dd>{km(reach)}</dd></>)}
        {radar > 0 && (<><dt>Radar</dt><dd>{km(radar)}</dd></>)}
        {live && state && state.total > 0 && (<><dt>Strength</dt><dd>{state.alive} of {state.total}</dd></>)}
        {site.cap && (<><dt>Capture radius</dt><dd>{Math.round(site.cap)} m</dd></>)}
        {schedule && live && (<><dt>Next column</dt><dd>{schedule.next_in[0] <= 0 ? "now" : clock(schedule.next_in[0])}–{clock(schedule.next_in[1])}</dd></>)}
        {site.route && (<><dt>Patrol speed</dt><dd>{Math.round(site.route.speed_kph)} km/h</dd></>)}
      </dl>
      {site.moves && site.cat !== "ship" && <p class="hint">This launcher moves between firing positions; the marker shows where it spawns.</p>}
      <ul class="units">
        {units.map(([u, slot]) => {
          const spec = c.units[u.u];
          const unit = state?.units.get(slot);
          const status: Status = unit?.[0] ?? "unknown";
          return (
            <li key={slot} class={status}>
              <div class="unit-head">
                <span class="unit-name">{spec?.n.replace(/"/g, "") ?? u.u}</span>
                <span class="unit-role">{(u.r ?? "").replace(/_/g, " ")}</span>
              </div>
              {spec?.w?.slice(0, 2).map((w) => (
                <div class="unit-weapon" key={w.n}>
                  {weaponLabel(w.n)}{w.max ? ` · ${km(w.max)}` : ""}{w.ammo ? ` · ${w.ammo} rounds` : ""}
                </div>
              ))}
              {live && (
                <div class="unit-state">
                  <span class={`state ${status} ${unit?.[1] ?? ""}`}>{stateText(status, unit?.[1] ?? "none", !!unit?.[2])}</span>
                  {unit && unit[3] !== null && <span class="mono dim">{unit[3] > 0 ? `may return in ${clock(unit[3])}` : "may be back"}</span>}
                  <span class="unit-actions">
                    {status !== "destroyed" ? (
                      <button onClick={() => mark(slot, true)}>Mark destroyed</button>
                    ) : (
                      <button onClick={() => mark(slot, false)}>Mark alive</button>
                    )}
                  </span>
                </div>
              )}
            </li>
          );
        })}
        {units.length === 0 && <li class="unknown"><span class="unit-role">No units here in this BR bracket.</span></li>}
      </ul>
      {live && units.length > 0 && (
        <footer>
          <button onClick={() => mark(null, true)}>Whole site destroyed</button>
          <button onClick={() => mark(null, false)}>Whole site alive</button>
          <button onClick={() => mark(null, null)}>Clear my marks</button>
        </footer>
      )}
    </section>
  );
}

const LAYERS: [string, string][] = [
  ["rings", "Weapon reach"],
  ["radar", "Radar reach"],
  ["friendly", "Friendly forces"],
  ["routes", "Column routes"],
  ["aircraft", "Aircraft"],
  ["ships", "Ships"],
  ["labels", "Names"],
  ["grid", "Grid"],
];

export function Layers() {
  const layers = prefs.value.layers;
  return (
    <div class="layers">
      {LAYERS.map(([key, text]) => (
        <button key={key} class={layers[key] ? "on" : ""} onClick={() => toggleLayer(key)} aria-pressed={layers[key]}>
          <i />
          {text}
        </button>
      ))}
    </div>
  );
}

const updateState = signal<string>("");
const appInfo = signal<{ version: string; pack_source: string } | null>(null);

function Num(props: { label: string; value: number; min: number; max: number; step: number; unit: string; onChange: (n: number) => void }) {
  return (
    <label class="field">
      <span>{props.label}</span>
      <input type="range" min={props.min} max={props.max} step={props.step} value={props.value} onChange={(e) => props.onChange(Number((e.target as HTMLInputElement).value))} />
      <output class="mono">{props.value} {props.unit}</output>
    </label>
  );
}

export function SettingsDialog() {
  if (!settingsOpen.value) return null;
  const s = snapshot.value, c = core.value, m = manifest.value, v = view.value;
  if (!s || !c) return null;
  const settings = s.settings;
  if (isDesktop && !appInfo.value) shell<{ version: string; pack_source: string }>("app_info").then((info) => (appInfo.value = info)).catch(() => {});
  const startDemo = () => command("demo_start", { level: v.level, variant: v.variant, era: v.era, team: v.team, speed: 1 }).then(() => (settingsOpen.value = false));
  const checkUpdate = async () => {
    updateState.value = "Checking…";
    try {
      const result = await shell<string>("update_data");
      updateState.value = result ?? "Updates are handled by the desktop app.";
    } catch (e) {
      updateState.value = String((e as Error).message ?? e);
    }
  };
  return (
    <div class="scrim" onClick={(e) => e.target === e.currentTarget && (settingsOpen.value = false)}>
      <div class="dialog" role="dialog" aria-label="Settings">
        <header>
          <h2>Settings</h2>
          <button class="close" onClick={() => (settingsOpen.value = false)} aria-label="Close">×</button>
        </header>

        <h3>Strike alerts</h3>
        <Num label="Blast radius to plan with" value={settings.strike_radius_m} min={300} max={5000} step={100} unit="m" onChange={(n) => applySettings({ strike_radius_m: n })} />
        <Num label="Point out groups of at least" value={settings.strike_min_units} min={3} max={30} step={1} unit="units" onChange={(n) => applySettings({ strike_min_units: n })} />
        <Num label="Warn before a column reaches a site" value={settings.alert_lead_s} min={30} max={300} step={10} unit="s" onChange={(n) => applySettings({ alert_lead_s: n })} />
        <p class="hint">
          The game files give each bomb's yield ({c.nukes.filter((n, i, all) => all.findIndex((x) => x.yield_kt === n.yield_kt) === i).map((n) => `${n.yield_kt} kt`).join(", ")}) but not
          its blast radius, so set the radius you see in game.
        </p>
        <label class="check">
          <input type="checkbox" checked={prefs.value.sound} onChange={() => setPref({ sound: !prefs.value.sound })} />
          Soft tone when a column reaches an enemy site
        </label>

        <h3>Game connection</h3>
        <label class="check">
          <input type="checkbox" checked={settings.colors_swapped} onChange={() => applySettings({ colors_swapped: !settings.colors_swapped })} />
          I use swapped team colours in game (enemies blue, friends red)
        </label>
        <p class="hint">
          The app only reads the page War Thunder itself serves at 127.0.0.1:8111 while it runs. It does not touch the
          game's memory or files and sends nothing to the internet.
        </p>
        <div class="buttons">
          {s.recording ? (
            <button onClick={() => command("record", { on: false })}>Stop recording</button>
          ) : (
            <button onClick={() => command("record", { on: true }).catch((e) => (updateState.value = String(e.message)))} disabled={!!s.demo}>Record this battle to a file</button>
          )}
          <button onClick={() => command("reset_battle")}>Forget this battle</button>
        </div>
        {s.recording && <p class="hint mono">{s.recording}</p>}

        <h3>Demo battle</h3>
        <p class="hint">Plays a simulated battle on the map chosen in the top bar, built from the real mission data. Useful to learn the app without the game.</p>
        <div class="buttons">
          {s.demo ? (
            <>
              <button onClick={() => command("demo_stop")}>Stop demo</button>
              {[1, 4, 10].map((speed) => (
                <button key={speed} class={s.demo?.speed === speed ? "on" : ""} onClick={() => command("demo_speed", { speed })}>{speed}× speed</button>
              ))}
            </>
          ) : (
            <button onClick={startDemo}>Start demo battle</button>
          )}
        </div>

        {isDesktop && (
          <>
            <h3>Mini map window</h3>
            <p class="hint">A small always-on-top window with nearby threats and alerts. Run the game in borderless or windowed mode to see it over the game.</p>
            <div class="buttons">
              <button onClick={() => shell("overlay", { action: "show" })}>Show</button>
              <button onClick={() => shell("overlay", { action: "hide" })}>Hide</button>
              <button onClick={() => shell("overlay", { action: "locked" })}>Let clicks pass through</button>
              <button onClick={() => shell("overlay", { action: "unlocked" })}>Make it movable again</button>
            </div>
          </>
        )}

        <h3>Map data</h3>
        <dl class="facts">
          <dt>Game version</dt><dd>{c.game_version}</dd>
          <dt>Built</dt><dd>{m?.generated?.slice(0, 10) ?? "—"}</dd>
          <dt>Datamine revision</dt><dd class="mono">{m?.datamine.commit.slice(0, 10) ?? "—"}</dd>
          {appInfo.value && (<><dt>App version</dt><dd>{appInfo.value.version} · {appInfo.value.pack_source} data</dd></>)}
        </dl>
        <div class="buttons">
          {isDesktop && <button onClick={checkUpdate}>Check for new map data</button>}
          {updateState.value && <span class="hint">{updateState.value}</span>}
        </div>
        <p class="hint">
          Positions, units and ranges come from the public War Thunder datamine (gszabi99/War-Thunder-Datamine). Terrain and
          roads come from majorlier's Nuclear Thunder Interactive Map. Not affiliated with Gaijin Entertainment.
        </p>
      </div>
    </div>
  );
}
