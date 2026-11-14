// The row floating at the top of the map: what battle this is, what to
// show, and the few global buttons.

import { signal } from "@preact/signals";
import { applySettings, core, FILTERS, isLive, pickFilter, prefs, setPref, snapshot, toggleLayer, view, level } from "../store";
import { coverStats, terrainReady } from "../map/cover";
import { br, clock } from "./format";
import { Icon, TEAM_NAME } from "./shared";

export const openPopover = signal<"battle" | "layers" | null>(null);
export const settingsOpen = signal(false);
const toggle = (id: "battle" | "layers") => (openPopover.value = openPopover.value === id ? null : id);

function Row(props: { on: boolean; onClick: () => void; children: any; note?: string }) {
  return (
    <button class={`pick ${props.on ? "on" : ""}`} onClick={props.onClick}>
      <span class="pick-mark" />
      <span>{props.children}</span>
      {props.note && <span class="pick-note">{props.note}</span>}
    </button>
  );
}

function BattlePopover() {
  const c = core.value!, s = snapshot.value, v = view.value;
  const live = isLive.value && !s?.unknown_map;
  const m = s?.settings;
  const battle = s?.battle;
  // In a battle these override detection; otherwise they are just what the map shows.
  const setLevel = (id: string | null) => (live ? applySettings({ manual_level: id }) : id && setPref({ level: id }));
  const setTeam = (team: number | null) => (live ? applySettings({ manual_team: team }) : team && setPref({ team }));
  const setLayout = (id: string | null) => (live ? applySettings({ manual_variant: id }) : id && setPref({ variant: id }));
  const setEra = (id: string | null) => (live ? applySettings({ manual_era: id }) : id && setPref({ era: id }));
  return (
    <div class="popover battle-pop material">
      <div class="pop-cols">
        <section>
          <h4>Map</h4>
          {live && <Row on={!m?.manual_level} onClick={() => setLevel(null)}>Automatic</Row>}
          {c.levels.map((l) => (
            <Row key={l.id} on={live ? m?.manual_level === l.id : v.level === l.id} onClick={() => setLevel(l.id)}>{l.name}</Row>
          ))}
          <h4>BR bracket</h4>
          {live && <Row on={!m?.manual_era} onClick={() => setEra(null)}>Automatic</Row>}
          {c.eras.map((e) => (
            <Row key={e.id} on={live ? m?.manual_era === e.id : v.era === e.id} onClick={() => setEra(e.id)} note={e.label}>{br(e.br)}</Row>
          ))}
        </section>
        <section>
          <h4>Your team</h4>
          {live && <Row on={!m?.manual_team} onClick={() => setTeam(null)}>Automatic</Row>}
          {[1, 2].map((team) => (
            <Row key={team} on={live ? m?.manual_team === team : v.team === team} onClick={() => setTeam(team)}>{TEAM_NAME[team]}</Row>
          ))}
          <h4>Starting positions</h4>
          {live && <Row on={!m?.manual_variant} onClick={() => setLayout(null)}>Automatic</Row>}
          <Row on={live ? m?.manual_variant === "standard" : v.variant === "standard"} onClick={() => setLayout("standard")}>Standard</Row>
          <Row on={live ? m?.manual_variant === "mirror" : v.variant === "mirror"} onClick={() => setLayout("mirror")} note="teams trade sides">Swapped</Row>
        </section>
      </div>
      {live && battle && battle.why.length > 0 && (
        <div class="why">
          <h4>How the app worked this out</h4>
          {battle.why.map((line, i) => <p key={i}>{line}</p>)}
          {s && !s.demo && (battle.side_conf === "guess" || battle.side_conf === "none") && (
            <p class="warn">Not sure about your side yet. If the colours look wrong, pick your team above.</p>
          )}
        </div>
      )}
    </div>
  );
}

function BattleCapsule() {
  const c = core.value, s = snapshot.value, v = view.value;
  if (!c) return null;
  const live = isLive.value;
  const state = !s ? "off" : s.demo ? "demo" : s.link;
  const lv = c.levels.find((l) => l.id === v.level);
  const era = c.eras.find((e) => e.id === v.era);
  const unsure = live && !s?.demo && !s?.unknown_map && (s?.battle.side_conf === "guess" || s?.battle.side_conf === "none");
  const status = !s ? "Starting…" : s.demo ? "Demo" : s.link === "battle" ? "Live" : s.link === "menu" ? "Waiting for a battle" : "Game not running";
  return (
    <div class="anchor">
      <button class={`capsule material ${openPopover.value === "battle" ? "open" : ""}`} onClick={() => toggle("battle")}>
        <span class={`dot ${state}`} />
        <span class="cap-status">{status}</span>
        <span class="cap-sep" />
        <span class="cap-main">{s?.unknown_map && live ? "Unknown map" : lv?.name ?? "—"}</span>
        <span class="cap-sub">{TEAM_NAME[v.team]}{v.variant === "mirror" ? " · swapped" : ""}</span>
        <span class="cap-sub">{era ? `BR ${br(era.br)}` : ""}</span>
        {unsure && <span class="cap-flag">check side</span>}
        {live && s?.battle.clock_s != null && <span class="cap-clock">{clock(s.battle.clock_s)}</span>}
        {Icon.chevron}
      </button>
      {openPopover.value === "battle" && <BattlePopover />}
    </div>
  );
}

function FilterPills() {
  const p = prefs.value;
  const all = p.show.length === 0;
  return (
    <div class="pills material" role="toolbar" aria-label="What to show on the map">
      <button class={all ? "on" : ""} onClick={() => pickFilter(null, false)}>All</button>
      {FILTERS.map((f) => (
        <button
          key={f.id}
          class={p.show.includes(f.id) ? "on" : ""}
          title={`Show only ${f.label.toLowerCase()}. Hold Ctrl to combine.`}
          onClick={(e) => pickFilter(f.id, e.ctrlKey || e.metaKey || e.shiftKey)}
        >
          {f.label}
        </button>
      ))}
      <span class="pill-sep" />
      <button class={p.enemyOnly ? "on enemy" : ""} title="Hide friendly forces" onClick={() => setPref({ enemyOnly: !p.enemyOnly })}>Enemy only</button>
    </div>
  );
}

const OVERLAYS: [string, string, string][] = [
  ["rings", "Weapon reach", "How far each side's air defence can shoot"],
  ["cover", "Terrain cover", "Where air defence can actually see you"],
  ["radar", "Radar reach", "Detection range of enemy radars"],
  ["routes", "Convoy routes", "The road ahead of each column"],
  ["aircraft", "Aircraft", "Other aircraft on the game's map"],
  ["labels", "Names", "Site names next to the symbols"],
  ["grid", "Grid", "Grid squares, as in the game"],
];

function LayersPopover() {
  const p = prefs.value, layers = p.layers;
  const hasTerrain = !!level.value?.terrain;
  const liveAlt = isLive.value ? snapshot.value?.player?.alt_m ?? null : null;
  const following = p.coverLive && liveAlt !== null;
  const stats = coverStats.value;
  return (
    <div class="popover layers-pop material right">
      {OVERLAYS.filter(([key]) => key !== "cover" || hasTerrain).map(([key, text, hint]) => (
        <button key={key} class="switch-row" onClick={() => toggleLayer(key)} aria-pressed={!!layers[key]}>
          <span>
            <b>{text}</b>
            <small>{hint}</small>
          </span>
          <i class={`switch ${layers[key] ? "on" : ""}`} />
        </button>
      ))}
      {layers.cover && hasTerrain && (
        <div class="cover-box">
          <div class="cover-key"><i class="exposed" /> exposed <i class="hidden" /> hidden by terrain</div>
          <label class="check">
            <input type="checkbox" checked={p.coverLive} onChange={() => setPref({ coverLive: !p.coverLive })} />
            Use my altitude in battle
          </label>
          {following ? (
            <div class="cover-alt">{Math.round(liveAlt!)} m above sea level</div>
          ) : (
            <>
              <input type="range" min={50} max={12000} step={50} value={p.coverAlt} onInput={(e) => setPref({ coverAlt: Number((e.target as HTMLInputElement).value) })} />
              <div class="seg small">
                <span class="cover-alt">{p.coverAlt} m</span>
                <button class={p.coverAgl ? "" : "on"} onClick={() => setPref({ coverAgl: false })}>above sea</button>
                <button class={p.coverAgl ? "on" : ""} onClick={() => setPref({ coverAgl: true })}>above ground</button>
              </div>
            </>
          )}
          {terrainReady.value !== level.value?.id ? (
            <small>Loading terrain…</small>
          ) : stats && stats.exposed + stats.hidden > 0 ? (
            <small>{Math.round((stats.hidden / (stats.exposed + stats.hidden)) * 100)}% of the area inside weapon reach is hidden by terrain.</small>
          ) : null}
        </div>
      )}
    </div>
  );
}

export function Bar() {
  const s = snapshot.value;
  return (
    <div class="bar">
      <BattleCapsule />
      <FilterPills />
      <div class="tools">
        {s?.recording && <span class="rec material">Recording</span>}
        <div class="anchor">
          <button class={`tool material ${openPopover.value === "layers" ? "open" : ""}`} title="Overlays" onClick={() => toggle("layers")}>{Icon.layers}</button>
          {openPopover.value === "layers" && <LayersPopover />}
        </div>
        <button class={`tool material ${prefs.value.rail ? "open" : ""}`} title="Side panel" onClick={() => setPref({ rail: !prefs.value.rail })}>{Icon.sidebar}</button>
        <button class="tool material" title="Settings" onClick={() => (settingsOpen.value = true)}>{Icon.gear}</button>
      </div>
    </div>
  );
}
