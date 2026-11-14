// Top strip: connection state, what battle this is and how sure the app is.

import { signal } from "@preact/signals";
import { applySettings, core, isLive, prefs, setPref, snapshot, view } from "../store";
import type { Conf } from "../types";
import { br, clock } from "./format";

const openMenu = signal<string | null>(null);
export const settingsOpen = signal(false);

const CONF_TEXT: Record<Conf, string> = { none: "", guess: "guess", likely: "likely", confirmed: "detected", manual: "set by you" };

function Chip(props: { id: string; label: string; value: string; conf?: Conf; children: any }) {
  const open = openMenu.value === props.id;
  return (
    <div class="chip-wrap">
      <button class={`chip ${open ? "open" : ""}`} onClick={() => (openMenu.value = open ? null : props.id)}>
        <span class="chip-label">{props.label}</span>
        <span class="chip-value">{props.value}</span>
        {props.conf && props.conf !== "none" && <span class={`conf ${props.conf}`}>{CONF_TEXT[props.conf]}</span>}
      </button>
      {open && <div class="menu" onClick={() => (openMenu.value = null)}>{props.children}</div>}
    </div>
  );
}

function Option(props: { on: boolean; onClick: () => void; children: any }) {
  return (
    <button class={`option ${props.on ? "on" : ""}`} onClick={props.onClick}>
      {props.children}
    </button>
  );
}

export function TopBar() {
  const c = core.value, s = snapshot.value, v = view.value;
  if (!c) return <header class="top" />;
  const live = isLive.value;
  const battle = s?.battle;
  const manual = s?.settings;
  const level = c.levels.find((l) => l.id === v.level);
  const era = c.eras.find((e) => e.id === v.era);
  const linkText = !s ? "Starting" : s.demo ? "Demo battle" : s.link === "battle" ? "In battle" : s.link === "menu" ? "Game running" : "Game not detected";
  const linkClass = !s ? "off" : s.demo ? "demo" : s.link;

  // Offline the choices are plain preferences; in a battle they override detection.
  const pickLevel = (id: string | null) => (live ? applySettings({ manual_level: id }) : id && setPref({ level: id }));
  const pickSide = (variant: string | null, team: number | null) =>
    live ? applySettings({ manual_variant: variant, manual_team: team }) : variant && team && setPref({ variant, team });
  const pickEra = (id: string | null) => (live ? applySettings({ manual_era: id }) : id && setPref({ era: id }));
  const sideText = `Team ${v.team} · ${v.variant === "mirror" ? "swapped" : "standard"}`;

  return (
    <header class="top">
      <div class="brand">
        <svg viewBox="0 0 32 32" width="18" height="18" aria-hidden="true">
          <path d="M16 3 29 16 16 29 3 16Z" fill="none" stroke="currentColor" stroke-width="2.6" />
          <circle cx="16" cy="16" r="3.4" fill="currentColor" />
        </svg>
        <span>Nuclear Thunder Live</span>
      </div>
      <div class={`link ${linkClass}`} title={battle?.why.join("\n") ?? ""}>
        <i />
        {linkText}
      </div>

      <Chip id="map" label="Map" value={level?.name ?? "—"} conf={live ? battle?.level_conf : undefined}>
        {live && <Option on={!manual?.manual_level} onClick={() => pickLevel(null)}>Detect automatically</Option>}
        {c.levels.map((l) => (
          <Option key={l.id} on={l.id === v.level && (!live || !!manual?.manual_level)} onClick={() => pickLevel(l.id)}>
            {l.name}
          </Option>
        ))}
      </Chip>

      <Chip id="side" label="Your side" value={sideText} conf={live ? battle?.side_conf : undefined}>
        {live && <Option on={!manual?.manual_team} onClick={() => pickSide(null, null)}>Detect automatically</Option>}
        {(["standard", "mirror"] as const).flatMap((variant) =>
          [1, 2].map((team) => (
            <Option key={variant + team} on={v.variant === variant && v.team === team && (!live || !!manual?.manual_team)} onClick={() => pickSide(variant, team)}>
              Team {team} ({team === 1 ? "USSR-led" : "NATO-led"}) · {variant === "mirror" ? "swapped positions" : "standard positions"}
            </Option>
          )),
        )}
      </Chip>

      <Chip id="era" label="BR bracket" value={era ? `${br(era.br)} · ${era.label}` : "—"} conf={live ? battle?.era_conf : undefined}>
        {live && <Option on={!manual?.manual_era} onClick={() => pickEra(null)}>Detect automatically</Option>}
        {c.eras.map((e) => (
          <Option key={e.id} on={e.id === v.era && (!live || !!manual?.manual_era)} onClick={() => pickEra(e.id)}>
            <span class="mono">{br(e.br)}</span> {e.label}
            {live && battle?.era_options.includes(e.id) && battle.era_conf === "guess" ? " · fits" : ""}
          </Option>
        ))}
      </Chip>

      {live && s?.player?.vehicle_name && (
        <div class="stat" title="Your vehicle, read from the game">
          <span class="chip-label">Vehicle</span>
          <span class="chip-value">
            {s.player.vehicle_name.replace(/^[^\p{L}\p{N}(]+/u, "")}
            {s.player.br ? <span class="mono dim"> {s.player.br.toFixed(1)}</span> : null}
          </span>
        </div>
      )}
      {live && battle && battle.clock_s !== null && (
        <div class="stat" title="Battle time, synchronised from the kill feed">
          <span class="chip-label">Clock</span>
          <span class="chip-value mono">{clock(battle.clock_s)}</span>
        </div>
      )}

      <div class="spacer" />
      {s?.recording && <div class="rec">REC</div>}
      <button class="tool" onClick={() => setPref({ rail: !prefs.value.rail })}>{prefs.value.rail ? "Hide panel" : "Show panel"}</button>
      <button class="tool" onClick={() => (settingsOpen.value = true)}>Settings</button>
    </header>
  );
}

export function closeMenus() {
  if (openMenu.value) openMenu.value = null;
}
