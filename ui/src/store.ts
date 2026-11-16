// App state. Signals keep rendering cheap: only what changed re-renders.

import { computed, effect, signal } from "@preact/signals";
import { command, packJson, subscribe } from "./transport";
import type { Core, Level, Manifest, Settings, Site, Snapshot, Status, Proof, Variant } from "./types";

export const core = signal<Core | null>(null);
export const manifest = signal<Manifest | null>(null);
export const snapshot = signal<Snapshot | null>(null);
export const level = signal<Level | null>(null);
export const loadError = signal<string | null>(null);
/** One-line message from the app itself (for example "new map data ready"). */
export const notice = signal<string | null>(null);
export const UNKNOWN_MAP = "_live";

/** Groups of site categories the map can be filtered by. */
export const FILTERS: { id: string; label: string; cats: string[] }[] = [
  { id: "sam", label: "SAM sites", cats: ["long_sam", "short_sam"] },
  { id: "spaa", label: "SPAA", cats: ["sam_cover", "spaa_cover"] },
  { id: "airfields", label: "Airfields", cats: ["airfield"] },
  { id: "depots", label: "Depots", cats: ["assembly_area", "fuel_factory", "ammo_factory"] },
  { id: "strongpoints", label: "Strongpoints", cats: ["strongpoint"] },
  { id: "radars", label: "Radars", cats: ["ew_radar"] },
  { id: "ships", label: "Ships", cats: ["ship"] },
  { id: "launchers", label: "Launchers", cats: ["mlrs", "tbm"] },
  { id: "convoys", label: "Convoys", cats: [] },
];

interface Prefs {
  level: string; variant: string; era: string; team: number;
  tab: "strike" | "defence" | "convoys" | "log";
  /** Overlays drawn on top of the sites. */
  layers: Record<string, boolean>;
  /** Filter ids to show; empty means everything. */
  show: string[];
  /** Hide friendly forces. */
  enemyOnly: boolean;
  sound: boolean; rail: boolean;
  /** Map symbols: the app's simple ones, or the game's War Thunder, NATO or USSR sets. */
  iconStyle: "simple" | "wt" | "nato" | "ussr";
  /** Keep the map centred on the player during a battle. */
  follow: boolean;
  /** Terrain cover: follow the player's altitude, or use the fixed one. */
  coverLive: boolean; coverAlt: number; coverAgl: boolean;
}
const defaults: Prefs = {
  level: "", variant: "standard", era: "", team: 1, tab: "strike",
  layers: { cover: false, rings: true, radar: false, routes: true, grid: true, labels: true, aircraft: true },
  show: [], enemyOnly: false,
  sound: true, rail: true, iconStyle: "nato", follow: false, coverLive: true, coverAlt: 500, coverAgl: false,
};
function readPrefs(): Prefs {
  try {
    const saved = JSON.parse(localStorage.getItem("ntl.prefs") || "{}");
    return { ...defaults, ...saved, layers: { ...defaults.layers, ...(saved.layers || {}) } };
  } catch {
    return defaults;
  }
}
export const prefs = signal<Prefs>(readPrefs());
effect(() => {
  try {
    localStorage.setItem("ntl.prefs", JSON.stringify(prefs.value));
  } catch {
    /* storage unavailable: preferences just do not persist */
  }
});
export const setPref = (patch: Partial<Prefs>) => (prefs.value = { ...prefs.value, ...patch });
export const toggleLayer = (key: string) => setPref({ layers: { ...prefs.value.layers, [key]: !prefs.value.layers[key] } });

/** Categories currently shown, or null when nothing is filtered out. */
export const shownCategories = computed<Set<string> | null>(() => {
  const show = prefs.value.show;
  if (!show.length) return null;
  return new Set(FILTERS.filter((f) => show.includes(f.id)).flatMap((f) => f.cats));
});
export const convoysShown = computed(() => !prefs.value.show.length || prefs.value.show.includes("convoys"));
export function siteShown(site: Site): boolean {
  const cats = shownCategories.value;
  if (prefs.value.enemyOnly && site.team !== enemyTeam.value) return false;
  return !cats || cats.has(site.cat);
}
/** Click = show only this; with Ctrl/Shift/Cmd = add or remove it. */
export function pickFilter(id: string | null, additive: boolean) {
  const current = prefs.value.show;
  if (id === null) return setPref({ show: [] });
  if (additive) return setPref({ show: current.includes(id) ? current.filter((x) => x !== id) : [...current, id] });
  setPref({ show: current.length === 1 && current[0] === id ? [] : [id] });
}

/** Selected site index in the current variant. */
export const selected = signal<number | null>(null);
export const hovered = signal<number | null>(null);
/** A single vehicle under the pointer (at close zoom): site index and slot. */
export const hoveredUnit = signal<{ site: number; slot: number } | null>(null);
/** Where the pointer is while hovering a site, in window pixels. */
export const hoverAt = signal<{ x: number; y: number } | null>(null);
/** Ask the map to centre on a world position. */
export const focusRequest = signal<{ p: [number, number]; zoom?: number; n: number } | null>(null);
let focusCount = 0;
export const focusOn = (p: [number, number], zoom?: number) => (focusRequest.value = { p, zoom, n: ++focusCount });

const live = computed(() => {
  const s = snapshot.value;
  return !!s && s.link === "battle" && (!!s.battle.level || s.unknown_map);
});
export const isLive = live;

/** What the map shows: the live battle when there is one, otherwise the user's pick. */
export const view = computed(() => {
  const s = snapshot.value;
  const p = prefs.value;
  const c = core.value;
  const fallbackLevel = p.level || c?.levels[0]?.id || "";
  const fallbackEra = p.era || c?.eras[c.eras.length - 1]?.id || "";
  if (s && s.link === "battle" && s.unknown_map) {
    // A map the data pack does not know: show the game's own picture of it.
    return { level: UNKNOWN_MAP, variant: "standard", era: fallbackEra, team: 1, live: true };
  }
  if (s && live.value) {
    return {
      level: s.battle.level!, variant: s.battle.variant ?? p.variant, era: s.battle.era ?? fallbackEra,
      team: s.battle.team ?? p.team, live: true,
    };
  }
  return { level: fallbackLevel, variant: p.variant, era: fallbackEra, team: p.team, live: false };
});

export const variant = computed<Variant | null>(() => {
  const l = level.value;
  if (!l) return null;
  return l.variants.find((v) => v.id === view.value.variant) ?? l.variants[0] ?? null;
});
export const eraIndex = computed(() => Math.max(0, core.value?.eras.findIndex((e) => e.id === view.value.era) ?? 0));

export interface SiteState { alive: number; total: number; destroyed: boolean; visible: number; units: Map<number, [Status, Proof, boolean, number | null]> }

/** Per-site roll-up of unit states for the current battle. */
export const siteStates = computed<Map<number, SiteState>>(() => {
  const out = new Map<number, SiteState>();
  const v = variant.value;
  if (!v) return out;
  const bit = eraIndex.value;
  v.sites.forEach((site, si) => {
    const total = site.units.filter((u) => (u.e >> bit) & 1).length;
    out.set(si, { alive: total, total, destroyed: false, visible: 0, units: new Map() });
  });
  const s = snapshot.value;
  if (s && live.value) {
    for (const [si, slot, status, proof, visible, eta] of s.units) {
      const state = out.get(si);
      if (!state) continue;
      state.units.set(slot, [status, proof, visible, eta]);
      if (status === "destroyed") state.alive -= 1;
      if (visible) state.visible += 1;
    }
    for (const state of out.values()) state.destroyed = state.total > 0 && state.alive <= 0;
  }
  return out;
});

export const enemyTeam = computed(() => 3 - view.value.team);

export function siteName(site: Site): string {
  const kind: Record<string, string> = {
    airfield: "Airfield", strongpoint: "Strongpoint", assembly_area: "Vehicle depot", fuel_factory: "Fuel depot",
    ammo_factory: "Ammo depot", long_sam: "Long-range SAM", short_sam: "SAM site", sam_cover: "SAM escort",
    spaa_cover: "SPAA post", ew_radar: "Early-warning radar", radar_pad: "Radar pad", ship: "Ship", mlrs: "MLRS",
    tbm: "Missile launcher", other: "Site",
  };
  if (site.id.includes("airfield_main")) return "Main airfield";
  const number = /_(\d+)$/.exec(site.id)?.[1];
  const base = kind[site.cat] ?? "Site";
  return number ? `${base} ${number}` : base;
}

export async function applySettings(patch: Partial<Settings>) {
  const current = snapshot.value?.settings;
  if (!current) return;
  await command("set_settings", { ...current, ...patch });
}

let started = false;
export function start() {
  if (started) return;
  started = true;
  packJson<Core>("core.json")
    .then((c) => (core.value = c))
    .catch((e) => (loadError.value = `The map data could not be loaded (${e.message}).`));
  packJson<Manifest>("manifest.json").then((m) => (manifest.value = m)).catch(() => {});
  subscribe((s) => (snapshot.value = s));
  (window as any).__TAURI__?.event?.listen("data-updated", (event: { payload: string }) => {
    notice.value = `New map data for game version ${event.payload} is ready. Restart the app to use it.`;
  });

  // Load the level file whenever the shown map changes.
  let loading = "";
  effect(() => {
    const id = view.value.level;
    const c = core.value;
    if (!c || !id || loading === id) return;
    if (id === UNKNOWN_MAP) {
      const bounds = snapshot.peek()?.bounds;
      if (!bounds) return;
      loading = id;
      selected.value = null;
      const empty = { id: "standard", mission: "", mirror: false, sites: [], supply_links: [], armor_links: [], mobile: [], bombers: [], escalation: [] };
      const blank: Level = { id, level: id, name: "Unknown map", bounds, water: 0, variants: [empty] };
      level.value = blank;
      command<number[]>("map_image")
        .then((bytes) => {
          if (loading === id) level.value = { ...blank, image_url: URL.createObjectURL(new Blob([new Uint8Array(bytes)])) };
        })
        .catch(() => {});
      return;
    }
    const ref = c.levels.find((l) => l.id === id);
    if (!ref) return;
    loading = id;
    selected.value = null;
    packJson<Level>(ref.file)
      .then((l) => {
        if (loading === id) level.value = l;
      })
      .catch((e) => (loadError.value = `Map "${ref.name}" could not be loaded (${e.message}).`));
  });
}
