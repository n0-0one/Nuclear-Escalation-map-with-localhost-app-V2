// The panel on the right: strike targets, air defence, convoys and the log,
// or the details of the site you clicked.

import { command } from "../transport";
import { core, enemyTeam, eraIndex, focusOn, hoverAt, hovered, hoveredUnit, isLive, prefs, selected, setPref, siteName, siteStates, snapshot, variant } from "../store";
import type { Alert, Convoy, Cue, Snapshot, Variant } from "../types";
import { bearing, clock, km, oclock, weaponLabel } from "./format";
import { Icon, MapSymbol, TEAM_NAME, UnitPicture, unitGroups } from "./shared";

const TABS = [["strike", "Strike"], ["defence", "Air defence"], ["convoys", "Convoys"], ["log", "Log"]] as const;

function go(site: number, v: Variant) {
  selected.value = site;
  focusOn(v.sites[site].p, 0.02);
}

const Empty = ({ children }: { children: any }) => <p class="empty">{children}</p>;

function AlertCard({ alert, v }: { alert: Alert; v: Variant }) {
  return (
    <button class={`card alert ${alert.kind} ${alert.phase}`} onClick={() => go(alert.site, v)}>
      <span class="when">{alert.phase === "now" ? "Now" : `In ${clock(alert.eta_s)}`}</span>
      <b>{alert.kind === "stack" ? "Convoy at an enemy site" : "Convoy attacking your site"}</b>
      <span>{siteName(v.sites[alert.site])} · {alert.count} enemy units within {km(alert.r)}</span>
      {alert.uncertain && <small>The convoy is not on the map right now; its position is estimated.</small>}
    </button>
  );
}

function Strike({ s, v }: { s: Snapshot; v: Variant }) {
  const player = s.player;
  return (
    <>
      <h3>Alerts</h3>
      {s.alerts.length === 0 && <Empty>No enemy convoy is near a depot or strongpoint.</Empty>}
      {s.alerts.map((alert) => <AlertCard key={alert.id} alert={alert} v={v} />)}
      <h3>Best targets</h3>
      {s.opportunities.length === 0 && <Empty>Nothing bunched up within {km(s.settings.strike_radius_m)} yet.</Empty>}
      {s.opportunities.map((item, i) => {
        const dx = player ? item.p[0] - player.p[0] : 0, dz = player ? item.p[1] - player.p[1] : 0;
        return (
          <button class="card count" key={i} onClick={() => (item.site !== null ? go(item.site, v) : focusOn(item.p, 0.03))}>
            <span class="big">{item.count}</span>
            <b>{item.site !== null ? siteName(v.sites[item.site]) : "Convoy on the road"}</b>
            <span>
              {item.convoys.length > 0 ? `with ${item.convoys.length} convoy${item.convoys.length > 1 ? "s" : ""}` : "fixed units"}
              {player && ` · ${km(Math.hypot(dx, dz))} · ${bearing((Math.atan2(dx, dz) * 180) / Math.PI)}`}
            </span>
            {item.friendly > 0 && <small class="friendly">{item.friendly} friendly unit{item.friendly > 1 ? "s" : ""} inside the radius</small>}
          </button>
        );
      })}
    </>
  );
}

function CueRow({ cue, index, v }: { cue: Cue; index: number; v: Variant }) {
  const site = v.sites[cue.site];
  const groups = unitGroups(site, cue.site);
  const launcher = groups.find((g) => g.total > g.destroyed && (g.spec?.eng ?? g.spec?.gun ?? 0) > 0) ?? groups[0];
  return (
    <button class={`card cue ${cue.inside ? "inside" : ""}`} onClick={() => go(cue.site, v)}>
      <span class="num">{index + 1}</span>
      <b>{siteName(site)}</b>
      <span>{launcher?.name ?? ""}{cue.alive < cue.total ? ` · ${cue.alive}/${cue.total} left` : ""}</span>
      <span class="data">
        <b>{km(cue.dist_m)}</b>
        <span>{bearing(cue.bearing)}{cue.grid ? ` · ${cue.grid}` : ""}</span>
        <span class={cue.inside ? "danger" : ""}>{cue.inside ? "in range" : cue.relative !== null ? oclock(cue.relative) : ""}</span>
      </span>
    </button>
  );
}

function Defence({ s, v }: { s: Snapshot; v: Variant }) {
  const enemy = enemyTeam.value;
  const states = siteStates.value;
  const hit = v.sites
    .map((site, si) => ({ site, si, state: states.get(si)! }))
    .filter(({ state }) => state && state.owner === enemy && state.total > 0 && state.alive < state.total)
    .sort((a, b) => a.state.alive / a.state.total - b.state.alive / b.state.total);
  return (
    <>
      <h3>Nearest live air defence</h3>
      {s.cues.length === 0 && <Empty>{s.player ? "No live enemy SAM site is known." : "Waiting for your position."}</Empty>}
      {s.cues.map((cue, i) => <CueRow key={cue.site} cue={cue} index={i} v={v} />)}
      {s.cues.length > 0 && <p class="hint">The numbers follow the game's ten aiming-point slots: click these sites in this order on the in-game tactical map.</p>}
      <h3>Destroyed or damaged</h3>
      {hit.length === 0 && <Empty>Nothing on the enemy side has been knocked out yet.</Empty>}
      {hit.map(({ site, si, state }) => (
        <button class={`card state ${state.destroyed || state.disarmed ? "dead" : ""}`} key={si} onClick={() => go(si, v)}>
          <b>{siteName(site)}</b>
          <span>
            {state.destroyed ? (site.cat === "ship" ? "Sunk" : "Destroyed") : state.disarmed ? "All weapons destroyed" : `${state.total - state.alive} of ${state.total} destroyed`}
          </span>
        </button>
      ))}
    </>
  );
}

function ConvoyRow({ convoy, v }: { convoy: Convoy; v: Variant }) {
  const state = { observed: "On the map", predicted: "Estimated", scheduled: "Expected" }[convoy.state];
  const from = convoy.from !== null ? siteName(v.sites[convoy.from]) : null;
  const to = convoy.to !== null ? siteName(v.sites[convoy.to]) : null;
  return (
    <button class={`card convoy ${convoy.state}`} onClick={() => focusOn(convoy.p, 0.03)}>
      <span class="when">{state}</span>
      <b>{convoy.kind === "armor" ? "Armoured column" : convoy.kind === "supply" ? "Supply convoy" : "Convoy"} · {convoy.n} vehicles</b>
      <span>{from && to ? `${from} → ${to}` : "Route not known yet"}{convoy.eta_s !== null && ` · arrives in ${clock(convoy.eta_s)}`}</span>
      {convoy.unseen_s !== null && <small>Last seen {clock(convoy.unseen_s)} ago; following its road.</small>}
      {convoy.state === "scheduled" && <small>Not seen yet. This depot sends a convoy on a fixed rhythm.</small>}
    </button>
  );
}

function Convoys({ s, v }: { s: Snapshot; v: Variant }) {
  const enemy = s.convoys.filter((c) => c.enemy);
  return (
    <>
      <h3>Enemy convoys</h3>
      {enemy.length === 0 && <Empty>No enemy convoy seen or expected yet.</Empty>}
      {enemy.map((convoy) => <ConvoyRow key={convoy.id} convoy={convoy} v={v} />)}
      {s.schedules.length > 0 && (
        <>
          <h3>Next departures</h3>
          <div class="list">
            {s.schedules.slice().sort((a, b) => a.next_in[0] - b.next_in[0]).map((item) => (
              <button class="line" key={item.site} onClick={() => go(item.site, v)}>
                <span>{siteName(v.sites[item.site])}</span>
                <span class="num">{item.next_in[0] <= 0 ? "now" : clock(item.next_in[0])}–{clock(item.next_in[1])}{item.locked ? "" : " ?"}</span>
              </button>
            ))}
          </div>
          <p class="hint">Fuel and ammo depots send a convoy every six minutes. A "?" means none has been seen from that depot yet, so the window is wide.</p>
        </>
      )}
    </>
  );
}

function Log({ s, v }: { s: Snapshot; v: Variant }) {
  return (
    <>
      <h3>Battle log</h3>
      {(s.suspects?.length ?? 0) > 0 && (
        <>
          <p class="hint">Kills the game reported where it is not known yet which unit it was. Places it can still be are marked "?" on the map (when there are a few); the app settles it once the others are seen alive.</p>
          <div class="list">
            {s.suspects!.map((sus, i) => {
              const first = sus.units[0];
              return (
                <button class="line log maybe" key={i} disabled={!first} onClick={() => first && go(first[0], v)}>
                  <span class="num">{clock(Math.max(0, (s.battle.clock_s ?? 0) - sus.ago_s))}</span>
                  <span>{sus.name} destroyed · {sus.count === 0 ? "no known place left" : sus.count === 1 ? "one place left" : `one of ${sus.count} places`}</span>
                </button>
              );
            })}
          </div>
        </>
      )}
      {s.log.length === 0 && <Empty>Nothing yet.</Empty>}
      <div class="list">
        {s.log.map((entry, i) => (
          <button class={`line log ${entry.kind}`} key={i} disabled={entry.site === null} onClick={() => entry.site !== null && go(entry.site, v)}>
            <span class="num">{entry.battle_t !== null ? clock(entry.battle_t) : "–"}</span>
            <span>{entry.text}</span>
          </button>
        ))}
      </div>
    </>
  );
}

/** Everything about one site. Shown when a site is clicked. */
function SiteDetail({ index, v }: { index: number; v: Variant }) {
  const site = v.sites[index];
  const c = core.value!;
  const live = isLive.value;
  const state = siteStates.value.get(index);
  const owner = state?.owner ?? site.team;
  const enemy = owner === enemyTeam.value;
  const groups = unitGroups(site, index);
  const reach = Math.max(0, ...groups.map((g) => g.spec?.eng ?? g.spec?.ai_eng ?? g.spec?.gun ?? 0), ...site.units.map((u) => u.tr ?? 0));
  const radar = Math.max(0, ...groups.map((g) => g.spec?.det ?? 0));
  const capturable = ["strongpoint", "assembly_area", "fuel_factory", "ammo_factory", "airfield"].includes(site.cat);
  const capture = (team: number | null) => command("capture", { site: index, ...(team !== null ? { team } : {}) });
  const myTeam = 3 - enemyTeam.value;
  const mark = (slot: number | null, destroyed: boolean | null) =>
    command("mark", { site: index, ...(slot !== null ? { slot } : {}), ...(destroyed !== null ? { destroyed } : {}) });
  const schedule = snapshot.value?.schedules.find((s) => s.site === index);
  void c;
  return (
    <div class="detail">
      <button class="back" onClick={() => (selected.value = null)}>{Icon.back} Back</button>
      <span class={`tag ${enemy ? "enemy" : "friendly"}`}>{enemy ? "Enemy" : "Friendly"} · {TEAM_NAME[owner]}</span>
      {state?.captured && <span class="tag captured">Captured{state.captureProof === "manual" ? " (marked by you)" : ""}</span>}
      <h2>{siteName(site)}</h2>
      {live && state && state.total > 0 && (
        <div class={`status ${state.destroyed || state.disarmed ? "dead" : state.alive < state.total ? "hit" : "ok"}`}>
          {state.destroyed
            ? site.cat === "ship" ? "Sunk" : "Destroyed"
            : state.disarmed
              ? "All weapons destroyed · no longer a threat"
              : state.alive < state.total ? `${state.total - state.alive} of ${state.total} destroyed` : state.visible > 0 ? "Intact · on the map now" : "No losses seen"}
        </div>
      )}
      {live && state?.ship && !state.ship.destroyed && (
        <p class="hint">
          {state.ship.state === "observed"
            ? "On the map now."
            : state.ship.state === "predicted"
              ? `Not on the map for ${clock(state.ship.unseen_s ?? 0)}; shown where it should be along its route.`
              : "Not seen yet; shown where it should be from the battle clock."}
        </p>
      )}
      <dl class="facts">
        {reach > 0 && (<><dt>Weapon reach</dt><dd>{km(reach)}</dd></>)}
        {radar > 0 && (<><dt>Radar</dt><dd>{km(radar)}</dd></>)}
        {site.cap && (<><dt>Capture radius</dt><dd>{Math.round(site.cap)} m</dd></>)}
        {schedule && live && (<><dt>Next convoy</dt><dd>{schedule.next_in[0] <= 0 ? "now" : clock(schedule.next_in[0])}–{clock(schedule.next_in[1])}</dd></>)}
        {site.route && (<><dt>Patrol speed</dt><dd>{Math.round(site.route.speed_kph)} km/h</dd></>)}
      </dl>
      {site.moves && site.cat !== "ship" && <p class="hint">This launcher moves between firing positions; the marker shows where it spawns.</p>}
      <div class="vehicles">
        {groups.map((g) => {
          const dead = g.destroyed >= g.total;
          const weapon = g.spec?.w?.[0];
          return (
            <div class={`vehicle ${dead ? "dead" : ""}`} key={g.id}>
              <UnitPicture spec={g.spec} dead={dead} enemy={enemy} />
              <div class="v-main">
                <b>{g.name}{g.total > 1 ? ` × ${g.total}` : ""}</b>
                {weapon && <span>{weaponLabel(weapon.n)}{weapon.max ? ` · ${km(weapon.max)}` : ""}</span>}
                {live && (
                  <span class={g.destroyed ? "lost" : g.onMap ? "seen" : ""}>
                    {g.destroyed ? `${g.destroyed} destroyed${g.likely ? " (probably)" : ""}` : g.onMap ? "On the map now" : "Not seen yet"}
                    {g.backIn !== null && ` · ${g.backIn > 0 ? `may return in ${clock(g.backIn)}` : "may be back"}`}
                  </span>
                )}
              </div>
              {live && (
                <button class="mini" onClick={() => g.slots.forEach((slot) => mark(slot, g.destroyed < g.total))}>
                  {g.destroyed < g.total ? "Mark destroyed" : "Mark alive"}
                </button>
              )}
            </div>
          );
        })}
        {groups.length === 0 && <Empty>No units here in this BR bracket.</Empty>}
      </div>
      {live && groups.length > 0 && (
        <div class="actions">
          <button onClick={() => mark(null, true)}>{site.cat === "ship" ? "Sunk" : "Whole site destroyed"}</button>
          <button onClick={() => mark(null, false)}>{site.cat === "ship" ? "Afloat" : "Whole site alive"}</button>
          <button onClick={() => mark(null, null)}>Clear my marks</button>
        </div>
      )}
      {live && capturable && (
        <>
          <h3>Who holds it</h3>
          <p class="hint">
            Columns of tanks take depots, strongpoints and airfields once their defenders are gone. The app notices when the
            other side's vehicles hold the positions; you can also set it here.
          </p>
          <div class="actions">
            {owner !== myTeam && <button onClick={() => capture(myTeam)}>Taken by your side</button>}
            {owner === myTeam && <button onClick={() => capture(3 - myTeam)}>Taken by the enemy</button>}
            {(state?.captured || state?.captureProof === "manual") && <button onClick={() => capture(null)}>Back to original owner</button>}
          </div>
        </>
      )}
    </div>
  );
}

export function Sidebar() {
  const s = snapshot.value, v = variant.value;
  const tab = prefs.value.tab;
  const live = isLive.value && s && v;
  if (!prefs.value.rail) return null;
  if (selected.value !== null && v && v.sites[selected.value]) {
    return (
      <aside class="side material">
        <div class="side-body"><SiteDetail index={selected.value} v={v} /></div>
      </aside>
    );
  }
  return (
    <aside class="side material">
      <nav class="seg">
        {TABS.map(([id, text]) => (
          <button key={id} class={tab === id ? "on" : ""} onClick={() => setPref({ tab: id })}>
            {text}
            {id === "strike" && s && s.alerts.length > 0 && <i>{s.alerts.length}</i>}
          </button>
        ))}
      </nav>
      <div class="side-body">
        {!live && (
          <div class="idle">
            <h2>{s?.link === "menu" ? "Waiting for a battle" : s?.link === "battle" ? "Reading the battle…" : "Game not running"}</h2>
            <p>
              {s?.link === "menu"
                ? "War Thunder is running. Live tracking starts by itself when you spawn in a Nuclear Escalation battle."
                : s?.link === "battle"
                  ? "Working out the map and your side from what the game shows."
                  : "Start War Thunder and the app connects by itself. Until then you can study any map: pick the map, team and BR bracket at the top left."}
            </p>
            <p>Hover a site to see what is there. Click it for details.</p>
          </div>
        )}
        {live && s.unknown_map && <p class="hint">This map is not in the app's data yet, so only what the game shows live is drawn. Check for new map data in Settings.</p>}
        {live && tab === "strike" && <Strike s={s} v={v} />}
        {live && tab === "defence" && <Defence s={s} v={v} />}
        {live && tab === "convoys" && <Convoys s={s} v={v} />}
        {live && tab === "log" && <Log s={s} v={v} />}
      </div>
    </aside>
  );
}

const ROLE_TEXT: Record<string, string> = {
  launcher: "Missile launcher", tracker_radar: "Tracking radar", detector_radar: "Search radar", radar: "Radar",
  spaa: "Anti-aircraft", vehicle: "Vehicle", ship: "Ship",
};

/** One vehicle under the pointer, at close zoom. */
function UnitHoverCard({ siteIndex, slot, at, v }: { siteIndex: number; slot: number; at: { x: number; y: number }; v: Variant }) {
  const site = v.sites[siteIndex];
  const placement = site?.units[slot];
  const c = core.value;
  if (!placement || !c) return null;
  const spec = c.units[placement.u];
  const enemy = (siteStates.value.get(siteIndex)?.owner ?? site.team) === enemyTeam.value;
  const unit = isLive.value ? siteStates.value.get(siteIndex)?.units.get(slot) : undefined;
  const dead = unit?.[0] === "destroyed";
  const player = isLive.value ? snapshot.value?.player : null;
  const reach = spec?.eng ?? spec?.ai_eng ?? spec?.gun ?? placement.tr ?? 0;
  const weapon = spec?.w?.[0];
  const status = !unit
    ? null
    : dead
      ? unit[1] === "likely" ? "Probably destroyed" : unit[1] === "manual" ? "Destroyed (marked by you)" : "Destroyed"
      : unit[2] ? "On the map now" : "Seen earlier";
  const width = 300;
  const left = at.x + 18 + width > window.innerWidth ? at.x - width - 18 : at.x + 18;
  const top = Math.max(8, Math.min(at.y - 20, window.innerHeight - 220));
  void eraIndex.value;
  return (
    <div class="hover unit-hover material" style={{ left: `${left}px`, top: `${top}px`, width: `${width}px` }}>
      <div class="unit-hover-head">
        <UnitPicture spec={spec} dead={dead} enemy={enemy} />
        <div class="unit-hover-text">
          <b class="hover-title">{(spec?.s ?? spec?.n ?? placement.u).replace(/"/g, "")}</b>
          <span class="hover-sub">{ROLE_TEXT[placement.r ?? ""] ?? (placement.r ?? "").replace(/_/g, " ")}</span>
          {spec?.s && spec.n !== spec.s && <span class="hover-sub dim full-name">{spec.n.replace(/"/g, "")}</span>}
          <span class="hover-sub dim">{siteName(site)}</span>
        </div>
        <MapSymbol type={spec?.it ?? "tank"} enemy={enemy} dead={dead} size={24} />
      </div>
      <dl class="facts compact">
        {weapon && (<><dt>Weapon</dt><dd>{weaponLabel(weapon.n)}</dd></>)}
        {reach > 0 && (<><dt>Reach</dt><dd>{km(reach)}</dd></>)}
        {spec?.det && (<><dt>Radar</dt><dd>{km(spec.det)}</dd></>)}
        {player && (<><dt>From you</dt><dd>{km(Math.hypot(placement.p[0] - player.p[0], placement.p[1] - player.p[1]))} · {bearing((Math.atan2(placement.p[0] - player.p[0], placement.p[1] - player.p[1]) * 180) / Math.PI)}</dd></>)}
      </dl>
      {status && <span class={`unit-status ${dead ? "dead" : unit?.[2] ? "seen" : ""}`}>{status}{unit?.[3] != null && dead ? ` · ${unit[3] > 0 ? `may return in ${clock(unit[3])}` : "may be back"}` : ""}</span>}
    </div>
  );
}

/** Follows the pointer while it rests on a site: what is there, at a glance. */
export function HoverCard() {
  const index = hovered.value, at = hoverAt.value, v = variant.value;
  const unitHover = hoveredUnit.value;
  if (unitHover && at && v) return <UnitHoverCard siteIndex={unitHover.site} slot={unitHover.slot} at={at} v={v} />;
  if (index === null || !at || !v || !v.sites[index]) return null;
  const site = v.sites[index];
  const live = isLive.value;
  const state = siteStates.value.get(index);
  const enemy = (state?.owner ?? site.team) === enemyTeam.value;
  const here = state?.p ?? site.p;
  const groups = unitGroups(site, index);
  const reach = Math.max(0, ...groups.map((g) => g.spec?.eng ?? g.spec?.ai_eng ?? g.spec?.gun ?? 0), ...site.units.map((u) => u.tr ?? 0));
  const player = isLive.value ? snapshot.value?.player : null;
  // keep the card inside the window
  const width = 280, height = 96 + Math.ceil(Math.max(1, groups.length) / 2) * 92;
  const left = at.x + 18 + width > window.innerWidth ? at.x - width - 18 : at.x + 18;
  const top = Math.max(8, Math.min(at.y - 20, window.innerHeight - height - 8));
  return (
    <div class="hover material" style={{ left: `${left}px`, top: `${top}px`, width: `${width}px` }}>
      <span class={`tag ${enemy ? "enemy" : "friendly"}`}>{enemy ? "Enemy" : "Friendly"}{state?.captured ? " · captured" : ""}</span>
      <b class="hover-title">{siteName(site)}</b>
      <span class="hover-sub">
        {player ? `${km(Math.hypot(here[0] - player.p[0], here[1] - player.p[1]))} from you · ` : ""}
        {reach > 0 ? `Reach ${km(reach)}` : ""}
        {live && state && state.total > 0 && (
          <em class={state.destroyed || state.disarmed ? "dead" : state.alive < state.total ? "hit" : ""}>
            {reach > 0 ? " · " : ""}
            {state.destroyed ? (site.cat === "ship" ? "Sunk" : "Destroyed") : state.disarmed ? "Weapons destroyed" : state.alive < state.total ? `${state.total - state.alive} of ${state.total} destroyed` : "Intact"}
          </em>
        )}
      </span>
      <div class="hover-grid">
        {groups.map((g) => {
          const dead = g.destroyed >= g.total;
          return (
            <div class={`tile ${dead ? "dead" : ""}`} key={g.id}>
              <UnitPicture spec={g.spec} dead={dead} enemy={enemy} />
              <span class="tile-name">{g.name}</span>
              <span class="tile-count">
                {g.total > 1 ? `× ${g.total}` : ""}
                {g.destroyed > 0 && <em>{g.total > 1 ? ` · ${g.destroyed} lost` : "lost"}</em>}
              </span>
            </div>
          );
        })}
      </div>
      {groups.length === 0 && <span class="hover-sub">No units here in this BR bracket.</span>}
    </div>
  );
}
