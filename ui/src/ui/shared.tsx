// Small pieces shared by the panels.

import { packUrl } from "../transport";
import { core, eraIndex, siteStates } from "../store";
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

export function UnitPicture({ spec, dead }: { spec: UnitSpec | undefined; dead?: boolean }) {
  const src = spec?.img ? packUrl(`units/${spec.img}.webp`) : null;
  return (
    <span class={`pic ${dead ? "dead" : ""} ${spec?.img_generic ? "generic" : ""}`}>
      {src && <img src={src} alt="" loading="lazy" draggable={false} />}
    </span>
  );
}

export const Icon = {
  gear: (
    <svg viewBox="0 0 20 20" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
      <circle cx="10" cy="10" r="2.6" />
      <path d="M10 2.2v2.1M10 15.7v2.1M2.2 10h2.1M15.7 10h2.1M4.5 4.5 6 6M14 14l1.5 1.5M4.5 15.5 6 14M14 6l1.5-1.5" />
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
