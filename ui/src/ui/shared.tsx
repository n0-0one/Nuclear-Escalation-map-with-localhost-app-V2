// Small pieces shared by the panels.

import { packUrl } from "../transport";
import { iconUri } from "../map/icons";
import { core, eraIndex, prefs, siteStates } from "../store";
import type { Site, UnitSpec } from "../types";

export const TEAM_NAME: Record<number, string> = { 1: "USSR & China", 2: "NATO & allies" };

export interface UnitGroup { id: string; spec: UnitSpec | undefined; name: string; total: number; destroyed: number; likely: number; onMap: number; slots: number[]; backIn: number | null }

/** A site's vehicles grouped by type, with how many of each are gone. */
export function unitGroups(site: Site, siteIndex: number): UnitGroup[] {
  const c = core.value;
  const bit = eraIndex.value;
  const state = siteStates.value.get(siteIndex);
  const groups = new Map<string, UnitGroup>();
  site.units.forEach((u, slot) => {
    if (!((u.e >> bit) & 1)) return;
    const spec = c?.units[u.u];
    let group = groups.get(u.u);
    if (!group) {
      group = { id: u.u, spec, name: (spec?.s ?? spec?.n ?? u.u).replace(/"/g, ""), total: 0, destroyed: 0, likely: 0, onMap: 0, slots: [], backIn: null };
      groups.set(u.u, group);
    }
    group.total += 1;
    group.slots.push(slot);
    const unit = state?.units.get(slot);
    if (unit?.[0] === "destroyed") {
      group.destroyed += 1;
      if (unit[1] === "likely") group.likely += 1;
      if (unit[3] !== null) group.backIn = group.backIn === null ? unit[3] : Math.min(group.backIn, unit[3]);
    }
    if (unit?.[2]) group.onMap += 1;
  });
  return [...groups.values()];
}

/** The game's map symbol for a unit, in the chosen style. */
export function MapSymbol({ type, enemy, dead, size = 26 }: { type: string; enemy: boolean; dead?: boolean; size?: number }) {
  const style = prefs.value.iconStyle;
  const src = iconUri(style === "simple" ? "nato" : style, type, enemy, dead);
  if (!src) return null;
  return <img class={`symbol ${style}`} src={src} alt="" width={size} height={size} draggable={false} />;
}

/**
 * A unit's picture: vehicle artwork when the game has some, otherwise the
 * game's tech-tree icon, otherwise its map symbol.
 */
export function UnitPicture({ spec, dead, enemy = true }: { spec: UnitSpec | undefined; dead?: boolean; enemy?: boolean }) {
  const art = spec?.img && !spec.img_generic ? packUrl(`units/${spec.img}.webp`) : null;
  const slot = !art && spec?.slot ? packUrl(`units/${spec.slot}.webp`) : null;
  return (
    <span class={`pic ${dead ? "dead" : ""} ${art ? "" : "small"}`}>
      {art && <img src={art} alt="" loading="lazy" draggable={false} />}
      {slot && <img src={slot} alt="" loading="lazy" draggable={false} />}
      {!art && !slot && <MapSymbol type={spec?.it ?? "tank"} enemy={enemy} dead={dead} size={30} />}
    </span>
  );
}

/** A cog with eight teeth, drawn here rather than borrowed from an icon set. */
function gearPath(): string {
  const teeth = 8, outer = 8.4, inner = 6.3, cx = 10, cy = 10;
  const points: string[] = [];
  for (let i = 0; i < teeth * 4; i++) {
    const tooth = Math.floor(i / 4), phase = i % 4;
    const base = (tooth / teeth) * Math.PI * 2;
    const step = (Math.PI * 2) / teeth;
    const angle = base + [-0.2, -0.13, 0.13, 0.2][phase] * step * 1.6;
    const r = phase === 1 || phase === 2 ? outer : inner;
    points.push(`${(cx + Math.cos(angle) * r).toFixed(2)},${(cy + Math.sin(angle) * r).toFixed(2)}`);
  }
  return `M${points.join("L")}Z`;
}
const GEAR = gearPath();

export const Icon = {
  gear: (
    <svg viewBox="0 0 20 20" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round">
      <path d={GEAR} />
      <circle cx="10" cy="10" r="2.7" />
    </svg>
  ),
  follow: (
    <svg viewBox="0 0 20 20" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">
      <circle cx="10" cy="10" r="5.2" />
      <circle cx="10" cy="10" r="1.6" fill="currentColor" stroke="none" />
      <path d="M10 1.8v2.6M10 15.6v2.6M1.8 10h2.6M15.6 10h2.6" />
    </svg>
  ),
  download: (
    <svg viewBox="0 0 20 20" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
      <path d="M10 3v9.5M6 8.8l4 4 4-4M4 16.5h12" />
    </svg>
  ),
  layers: (
    <svg viewBox="0 0 20 20" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round">
      <path d="M10 3 2.5 7 10 11l7.5-4L10 3Z" />
      <path d="M2.5 10.5 10 14.5l7.5-4M2.5 13.8 10 17.8l7.5-4" opacity=".55" />
    </svg>
  ),
  sidebar: (
    <svg viewBox="0 0 20 20" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round">
      <rect x="2.5" y="4" width="15" height="12" rx="2.5" />
      <path d="M12.5 4v12" />
    </svg>
  ),
  close: (
    <svg viewBox="0 0 20 20" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
      <path d="m5 5 10 10M15 5 5 15" />
    </svg>
  ),
  chevron: (
    <svg viewBox="0 0 20 20" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="m5 7.5 5 5 5-5" />
    </svg>
  ),
  back: (
    <svg viewBox="0 0 20 20" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="m12 4-6 6 6 6" />
    </svg>
  ),
};
