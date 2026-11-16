// The game's own map symbols (War Thunder, NATO and USSR styles), painted in
// the team colour and cached as small bitmaps so the map can draw hundreds
// of them cheaply.

import { signal } from "@preact/signals";
import { packJson } from "../transport";

export type IconStyle = "simple" | "wt" | "nato" | "ussr";
type Icons = Record<"wt" | "nato" | "ussr", Record<string, string>>;

let icons: Icons | null = null;
/** Bumped when icon artwork finishes loading, so the map can redraw. */
export const iconVersion = signal(0);

export const ICON_FILL = { enemy: "#ff8a80", friendly: "#7cc4ff", dead: "#8e8e93" };

/** The symbol each kind of site uses, in NATO/USSR terms. */
export const SITE_ICON: Record<string, string> = {
  fuel_factory: "fuel_storage",
  ammo_factory: "ammo_storage",
  assembly_area: "tank_storage",
  strongpoint: "pillbox",
  long_sam: "sam",
  short_sam: "sam",
  sam_cover: "spaa",
  spaa_cover: "spaa",
  ew_radar: "radar_station",
  mlrs: "mlrs",
  tbm: "tbm",
};

const WT_NAME: Record<string, string> = {
  tank: "tracked", ifv: "light_tank", truck: "wheeled", apc: "light_tank",
  sam: "airdefence", spaa: "spaa", radar_station: "spaa",
  fuel_storage: "structure", ammo_storage: "structure", tank_storage: "structure", pillbox: "structure",
  mlrs: "tank_destroyer", tbm: "tank_destroyer", tank_destroyer: "tank_destroyer", artillery: "tank_destroyer",
};
const NAVAL = new Set(["aircraft_carrier", "battleship", "missile_cruiser", "heavy_cruiser", "cruiser", "destroyer", "frigate", "corvette", "boat", "ship"]);

export function loadIcons() {
  if (icons) return;
  packJson<Icons>("icons.json")
    .then((value) => {
      icons = value;
      iconVersion.value++;
    })
    .catch(() => {});
}

/** SVG source of a symbol, or null when the style has no such symbol. */
function source(style: Exclude<IconStyle, "simple">, type: string, enemy: boolean): string | null {
  if (!icons) return null;
  if (style === "wt") {
    const name = NAVAL.has(type) ? "ship" : WT_NAME[type] ?? type;
    return icons.wt[name] ?? icons.wt.tracked ?? null;
  }
  if (style === "nato") {
    const side = enemy ? "enemy" : "ally";
    return icons.nato[`${side}_${type}`] ?? (NAVAL.has(type) ? icons.nato[`${side}_ship`] : null) ?? icons.nato[`${side}_tank`] ?? null;
  }
  const ussrName: Record<string, string> = { apc: "ifv" };
  return icons.ussr[ussrName[type] ?? type] ?? (NAVAL.has(type) ? icons.ussr.ship : null) ?? icons.ussr.tank ?? null;
}

/** A data URI of the symbol, for use in <img>. */
export function iconUri(style: IconStyle, type: string, enemy: boolean, dead = false): string | null {
  const svg = source(style === "simple" ? "nato" : style, type, enemy);
  if (!svg) return null;
  const fill = dead ? ICON_FILL.dead : enemy ? ICON_FILL.enemy : ICON_FILL.friendly;
  let painted = svg.replace(/#fdfcff|#ffffff\b|#fff\b|\bwhite\b/gi, fill);
  if (style === "wt") {
    // The tiny War Thunder symbols get a thin dark keyline to stay readable.
    painted = painted.replace(new RegExp(`fill="${fill}"`, "gi"), `fill="${fill}" stroke="#000" stroke-width="0.45" paint-order="stroke fill"`);
  }
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(painted)}`;
}

const bitmaps = new Map<string, HTMLCanvasElement | "loading" | "failed">();

/**
 * A ready-to-draw bitmap of a symbol about `size` CSS pixels across, or null
 * while it is still being prepared (the map redraws once it is ready).
 */
export function iconBitmap(style: Exclude<IconStyle, "simple">, type: string, enemy: boolean, dead: boolean, size: number, dpr: number): HTMLCanvasElement | null {
  const key = `${style}|${type}|${enemy}|${dead}|${size}|${dpr}`;
  const cached = bitmaps.get(key);
  if (cached instanceof HTMLCanvasElement) return cached;
  if (cached) return null;
  const uri = iconUri(style, type, enemy, dead);
  if (!uri) return null;
  bitmaps.set(key, "loading");
  const image = new Image();
  image.onload = () => {
    // Keep the symbol's own proportions inside a size x size box.
    const scale = (size * dpr) / Math.max(image.naturalWidth || 1, image.naturalHeight || 1);
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round((image.naturalWidth || size) * scale));
    canvas.height = Math.max(1, Math.round((image.naturalHeight || size) * scale));
    const ctx = canvas.getContext("2d")!;
    ctx.imageSmoothingEnabled = style !== "wt"; // pixel-art symbols stay crisp
    ctx.drawImage(image, 0, 0, canvas.width, canvas.height);
    bitmaps.set(key, canvas);
    iconVersion.value++;
  };
  image.onerror = () => bitmaps.set(key, "failed");
  image.src = uri;
  return null;
}
