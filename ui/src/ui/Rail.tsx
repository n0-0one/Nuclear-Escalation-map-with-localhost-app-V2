// Right-hand panel: strike opportunities, nearest air defences, convoys, log.

import { core, enemyTeam, focusOn, isLive, prefs, selected, setPref, siteName, siteStates, snapshot, variant } from "../store";
import type { Alert, Convoy, Cue, Snapshot, Variant } from "../types";
import { bearing, clock, km, oclock } from "./format";

const TABS = [
  ["strike", "Strike"],
  ["defence", "Air defence"],
  ["convoys", "Convoys"],
  ["log", "Log"],
] as const;

function go(site: number, v: Variant) {
  selected.value = site;
  focusOn(v.sites[site].p, 0.02);
}

function Empty({ children }: { children: any }) {
  return <p class="empty">{children}</p>;
}

function AlertCard({ alert, v }: { alert: Alert; v: Variant }) {
  const site = v.sites[alert.site];
  const title = alert.kind === "stack" ? "Column at enemy site" : "Column attacking your site";
  return (
    <button class={`card alert ${alert.kind} ${alert.phase}`} onClick={() => go(alert.site, v)}>
      <span class="card-kicker">{alert.phase === "now" ? "Now" : `In ${clock(alert.eta_s)}`}</span>
      <span class="card-title">{title}</span>
      <span class="card-line">
        {siteName(site)} · <b>{alert.count}</b> enemy units inside {km(alert.r)}
      </span>
      {alert.uncertain && <span class="card-note">Column not on the map right now; position is estimated.</span>}
    </button>
  );
}

function Strike({ s, v }: { s: Snapshot; v: Variant }) {
  const player = s.player;
  return (
    <>
      <h3>Alerts</h3>
      {s.alerts.length === 0 && <Empty>No enemy column is near a depot or strongpoint.</Empty>}
      {s.alerts.map((alert) => <AlertCard key={alert.id} alert={alert} v={v} />)}
      <h3>Best targets</h3>
      {s.opportunities.length === 0 && <Empty>Nothing bunched up within {km(s.settings.strike_radius_m)} yet.</Empty>}
      {s.opportunities.map((item, i) => {
        const dx = player ? item.p[0] - player.p[0] : 0, dz = player ? item.p[1] - player.p[1] : 0;
        return (
          <button class="card" key={i} onClick={() => (item.site !== null ? go(item.site, v) : focusOn(item.p, 0.03))}>
            <span class="card-count">{item.count}</span>
            <span class="card-title">{item.site !== null ? siteName(v.sites[item.site]) : "Column on the road"}</span>
            <span class="card-line">
              {item.convoys.length > 0 ? `includes ${item.convoys.length} column${item.convoys.length > 1 ? "s" : ""}` : "fixed units only"}
              {player && ` · ${km(Math.hypot(dx, dz))} · ${bearing((Math.atan2(dx, dz) * 180) / Math.PI)}`}
            </span>
            {item.friendly > 0 && <span class="card-note warn">{item.friendly} friendly unit{item.friendly > 1 ? "s" : ""} inside the radius</span>}
          </button>
        );
      })}
    </>
  );
}

function CueRow({ cue, index, v }: { cue: Cue; index: number; v: Variant }) {
  const site = v.sites[cue.site];
  const c = core.value!;
  const bit = Math.max(0, c.eras.findIndex((e) => e.id === snapshot.value?.battle.era));
  const state = siteStates.value.get(cue.site);
  const names = new Set<string>();
  site.units.forEach((u, slot) => {
    if (((u.e >> bit) & 1) && u.r === "launcher" && state?.units.get(slot)?.[0] !== "destroyed") names.add(c.units[u.u]?.s ?? c.units[u.u]?.n ?? u.u);
  });
  if (names.size === 0) site.units.forEach((u) => ((u.e >> bit) & 1) && names.add(c.units[u.u]?.s ?? c.units[u.u]?.n ?? u.u));
  return (
    <button class={`cue ${cue.inside ? "inside" : ""}`} onClick={() => go(cue.site, v)}>
      <span class="cue-n">{index + 1}</span>
      <span class="cue-main">
        <span class="card-title">{siteName(site)}</span>
        <span class="card-line">{[...names].slice(0, 2).join(", ").replace(/"/g, "")}</span>
      </span>
      <span class="cue-data">
        <b>{km(cue.dist_m)}</b>
        <span>{bearing(cue.bearing)}{cue.grid ? ` · ${cue.grid}` : ""}</span>
        <span>{cue.inside ? "IN RANGE" : cue.relative !== null ? oclock(cue.relative) : `reach ${km(cue.reach_m)}`}</span>
      </span>
    </button>
  );
}

function Defence({ s, v }: { s: Snapshot; v: Variant }) {
  return (
    <>
      <h3>Nearest live air defences</h3>
      {s.cues.length === 0 && <Empty>{s.player ? "No live enemy SAM site is known." : "Waiting for your position."}</Empty>}
      {s.cues.map((cue, i) => <CueRow key={cue.site} cue={cue} index={i} v={v} />)}
      {s.cues.length > 0 && (
        <p class="hint">
          Numbers match the game's ten aiming-point slots. Open the tactical map in game and click these sites in this
          order; the game does not let other programs place aiming points for you.
        </p>
      )}
    </>
  );
}

function ConvoyRow({ convoy, v }: { convoy: Convoy; v: Variant }) {
  const state = { observed: "On map", predicted: "Estimated", scheduled: "Expected" }[convoy.state];
  const from = convoy.from !== null ? siteName(v.sites[convoy.from]) : null;
  const to = convoy.to !== null ? siteName(v.sites[convoy.to]) : null;
  return (
    <button class={`card convoy ${convoy.state}`} onClick={() => focusOn(convoy.p, 0.03)}>
      <span class="card-kicker">{state}</span>
      <span class="card-title">
        {convoy.kind === "armor" ? "Armoured column" : convoy.kind === "supply" ? "Supply column" : "Column"} · {convoy.n} vehicles
      </span>
      <span class="card-line">
        {from && to ? `${from} → ${to}` : "Route not known yet"}
        {convoy.eta_s !== null && ` · arrives in ${clock(convoy.eta_s)}`}
      </span>
      {convoy.unseen_s !== null && <span class="card-note">Last seen {clock(convoy.unseen_s)} ago; following its road.</span>}
      {convoy.state === "scheduled" && <span class="card-note">Not seen. This depot sends a column on a fixed rhythm.</span>}
    </button>
  );
}

function Convoys({ s, v }: { s: Snapshot; v: Variant }) {
  const enemy = s.convoys.filter((c) => c.enemy);
  const friendly = s.convoys.filter((c) => !c.enemy);
  return (
    <>
      <h3>Enemy columns</h3>
      {enemy.length === 0 && <Empty>No enemy column seen or expected yet.</Empty>}
      {enemy.map((convoy) => <ConvoyRow key={convoy.id} convoy={convoy} v={v} />)}
      {s.schedules.length > 0 && (
        <>
          <h3>Next departures</h3>
          {s.schedules.slice().sort((a, b) => a.next_in[0] - b.next_in[0]).map((item) => (
            <button class="row" key={item.site} onClick={() => go(item.site, v)}>
              <span>{siteName(v.sites[item.site])}</span>
              <span class="mono">
                {item.next_in[0] <= 0 ? "now" : clock(item.next_in[0])}–{clock(item.next_in[1])}
                {item.locked ? "" : " ?"}
              </span>
            </button>
          ))}
          <p class="hint">Fuel and ammo depots send a column every six minutes. "?" means no column from that depot has been seen yet, so the window is wide.</p>
        </>
      )}
      {friendly.length > 0 && <h3>Friendly columns</h3>}
      {friendly.map((convoy) => <ConvoyRow key={convoy.id} convoy={convoy} v={v} />)}
    </>
  );
}

function Log({ s, v }: { s: Snapshot; v: Variant }) {
  return (
    <>
      <h3>Battle log</h3>
      {s.battle.unlocated_kills > 0 && <p class="hint">{s.battle.unlocated_kills} kill{s.battle.unlocated_kills > 1 ? "s" : ""} reported for units whose position is not known yet.</p>}
      {s.log.length === 0 && <Empty>Nothing yet.</Empty>}
      {s.log.map((entry, i) => (
        <button class={`log ${entry.kind}`} key={i} disabled={entry.site === null} onClick={() => entry.site !== null && go(entry.site, v)}>
          <span class="mono">{entry.battle_t !== null ? clock(entry.battle_t) : "--:--"}</span>
          <span>{entry.text}</span>
        </button>
      ))}
    </>
  );
}

export function Rail() {
  const s = snapshot.value, v = variant.value;
  const tab = prefs.value.tab;
  const live = isLive.value && s && v;
  enemyTeam.value;
  return (
    <aside class="rail">
      <nav class="tabs">
        {TABS.map(([id, text]) => (
          <button key={id} class={tab === id ? "on" : ""} onClick={() => setPref({ tab: id })}>
            {text}
            {id === "strike" && s && s.alerts.length > 0 && <i class="badge">{s.alerts.length}</i>}
          </button>
        ))}
      </nav>
      <div class="rail-body">
        {!live && (
          <div class="idle">
            <h3>{s?.link === "menu" ? "Waiting for a battle" : s?.link === "battle" ? "Reading the battle…" : "Game not detected"}</h3>
            <p>
              {s?.link === "menu"
                ? "War Thunder is running. Live tracking starts by itself when you spawn in a Nuclear Escalation battle."
                : s?.link === "battle"
                  ? s.unknown_map
                    ? "This map is not in the data pack yet. Check for a data update in Settings."
                    : "Working out the map and your side from what the game shows."
                  : "Start War Thunder and the app connects by itself. Until then you can study any map here: pick the map, side and BR bracket in the top bar."}
            </p>
            <p>Want to see everything working without the game? Start the demo battle from Settings.</p>
          </div>
        )}
        {live && s.unknown_map && (
          <p class="hint">
            This map is not in the app's data yet, so only what the game shows live is drawn. Check for new map data in Settings.
          </p>
        )}
        {live && tab === "strike" && <Strike s={s} v={v} />}
        {live && tab === "defence" && <Defence s={s} v={v} />}
        {live && tab === "convoys" && <Convoys s={s} v={v} />}
        {live && tab === "log" && <Log s={s} v={v} />}
      </div>
    </aside>
  );
}
