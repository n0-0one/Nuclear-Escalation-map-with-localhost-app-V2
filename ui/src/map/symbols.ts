// Map symbols. Side is shown by frame shape as well as colour (hostile =
// diamond, friendly = rectangle), following the usual military map
// convention, so the map stays readable for colour-blind players.

export const COLOR = {
  hostile: "#ff453a",
  friendly: "#0a84ff",
  unknown: "#ffd60a",
  amber: "#ff9f0a",
  poi: "#64d2ff",
  aim: "#ffd60a",
  ink: "#f2f2f7",
  dim: "#8e8e93",
  panel: "#1c1c1e",
};
export const FONT = '"Segoe UI Variable Text", "Segoe UI", -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", Arial, sans-serif';

export const sideColor = (enemy: boolean) => (enemy ? COLOR.hostile : COLOR.friendly);

export function rgba(hex: string, alpha: number): string {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${n >> 16},${(n >> 8) & 255},${n & 255},${alpha})`;
}

/** Sites drawn small: single vehicles guarding something bigger. */
export const isMinor = (cat: string) => cat === "sam_cover" || cat === "spaa_cover" || cat === "radar_pad";

export interface SymbolOptions {
  enemy: boolean;
  cat: string;
  size: number;
  /** 0..1 share of units still alive; null when not tracked. */
  strength: number | null;
  total: number;
  destroyed: boolean;
  selected: boolean;
  hovered: boolean;
  /** At least one unit of the site is on the game's map right now. */
  seen: boolean;
}

function frame(ctx: CanvasRenderingContext2D, x: number, y: number, r: number, enemy: boolean) {
  ctx.beginPath();
  if (enemy) {
    ctx.moveTo(x, y - r);
    ctx.lineTo(x + r, y);
    ctx.lineTo(x, y + r);
    ctx.lineTo(x - r, y);
    ctx.closePath();
  } else {
    const w = r * 0.92, h = r * 0.7;
    ctx.rect(x - w, y - h, w * 2, h * 2);
  }
}

function glyph(ctx: CanvasRenderingContext2D, x: number, y: number, r: number, cat: string) {
  const g = r * 0.42;
  ctx.beginPath();
  switch (cat) {
    case "long_sam":
    case "short_sam": {
      // air-defence dome with a missile; range ticks mark the long-range sites
      ctx.arc(x, y + g * 1.1, g * 1.05, Math.PI, 0);
      ctx.moveTo(x, y + g * 0.9);
      ctx.lineTo(x, y - g * 1.15);
      ctx.moveTo(x - g * 0.42, y - g * 0.6);
      ctx.lineTo(x, y - g * 1.2);
      ctx.lineTo(x + g * 0.42, y - g * 0.6);
      if (cat === "long_sam") {
        ctx.moveTo(x - g * 0.5, y - g * 0.05);
        ctx.lineTo(x + g * 0.5, y - g * 0.05);
        ctx.moveTo(x - g * 0.5, y + g * 0.4);
        ctx.lineTo(x + g * 0.5, y + g * 0.4);
      }
      ctx.stroke();
      return;
    }
    case "sam_cover":
    case "spaa_cover":
      ctx.arc(x, y + g * 0.7, g * 1.0, Math.PI, 0);
      ctx.stroke();
      return;
    case "ew_radar":
      ctx.arc(x - g * 0.2, y + g * 0.2, g * 1.1, -Math.PI * 0.85, -Math.PI * 0.05);
      ctx.moveTo(x - g * 0.2, y + g * 0.2);
      ctx.lineTo(x + g * 0.75, y - g * 0.85);
      ctx.stroke();
      return;
    case "airfield":
      ctx.moveTo(x, y - g * 1.2);
      ctx.lineTo(x, y + g * 1.2);
      ctx.moveTo(x - g * 1.25, y - g * 0.1);
      ctx.lineTo(x + g * 1.25, y - g * 0.1);
      ctx.moveTo(x - g * 0.55, y + g * 1.0);
      ctx.lineTo(x + g * 0.55, y + g * 1.0);
      ctx.stroke();
      return;
    case "strongpoint":
      ctx.rect(x - g * 0.7, y - g * 0.7, g * 1.4, g * 1.4);
      ctx.fill();
      return;
    case "ship":
      ctx.moveTo(x - g * 1.3, y - g * 0.2);
      ctx.lineTo(x + g * 1.3, y - g * 0.2);
      ctx.lineTo(x + g * 0.8, y + g * 0.7);
      ctx.lineTo(x - g * 0.8, y + g * 0.7);
      ctx.closePath();
      ctx.fill();
      return;
    default: {
      const letter: Record<string, string> = { fuel_factory: "F", ammo_factory: "A", assembly_area: "V", mlrs: "M", tbm: "T" };
      const text = letter[cat];
      if (!text) return;
      ctx.font = `700 ${Math.round(r * 0.95)}px ${FONT}`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(text, x, y + r * 0.04);
    }
  }
}

/** Selection ring, destroyed cross and losses badge, drawn around any site symbol. */
export function drawSiteMarks(ctx: CanvasRenderingContext2D, x: number, y: number, r: number, o: SymbolOptions) {
  if (o.selected || o.hovered) {
    ctx.beginPath();
    ctx.arc(x, y, r + 7, 0, Math.PI * 2);
    ctx.strokeStyle = o.selected ? COLOR.amber : rgba(COLOR.ink, 0.6);
    ctx.lineWidth = o.selected ? 2.25 : 1.25;
    ctx.stroke();
  }
  if (o.destroyed) {
    // a clear red cross: this site is out of the fight
    ctx.beginPath();
    ctx.moveTo(x - r * 0.85, y - r * 0.85);
    ctx.lineTo(x + r * 0.85, y + r * 0.85);
    ctx.moveTo(x + r * 0.85, y - r * 0.85);
    ctx.lineTo(x - r * 0.85, y + r * 0.85);
    ctx.strokeStyle = rgba(COLOR.panel, 0.9);
    ctx.lineWidth = 5;
    ctx.stroke();
    ctx.strokeStyle = COLOR.hostile;
    ctx.lineWidth = 2.5;
    ctx.stroke();
  } else if (o.strength !== null && o.strength < 0.999 && o.total > 0) {
    // how much of the site is left, e.g. 3/6
    const alive = Math.round(o.strength * o.total);
    const text = `${alive}/${o.total}`;
    ctx.font = `700 10.5px ${FONT}`;
    const w = ctx.measureText(text).width + 8;
    const left = x + r * 0.55, top = y - r - 7;
    ctx.beginPath();
    ctx.roundRect(left, top, w, 15, 7.5);
    ctx.fillStyle = COLOR.amber;
    ctx.fill();
    ctx.fillStyle = "#1c1c1e";
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    ctx.fillText(text, left + 4, top + 8);
  }
}

/** The app's own simple symbols: frame shape shows the side, a glyph the kind. */
export function drawSite(ctx: CanvasRenderingContext2D, x: number, y: number, o: SymbolOptions) {
  const r = o.size;
  const color = sideColor(o.enemy);
  frame(ctx, x, y, r, o.enemy);
  ctx.fillStyle = rgba(COLOR.panel, o.destroyed ? 0.6 : 0.9);
  ctx.fill();
  ctx.strokeStyle = o.destroyed ? rgba(COLOR.dim, 0.8) : color;
  ctx.lineWidth = o.seen ? 2.25 : 1.5;
  ctx.stroke();
  ctx.strokeStyle = ctx.fillStyle = rgba(COLOR.ink, o.destroyed ? 0.35 : 0.95);
  ctx.lineWidth = 1.4;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  glyph(ctx, x, y, r, o.cat);
  drawSiteMarks(ctx, x, y, r, o);
}

/** An airfield as a runway bar along its longest runway. */
export function drawRunwayMark(ctx: CanvasRenderingContext2D, x: number, y: number, angle: number, enemy: boolean, destroyed: boolean) {
  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(angle);
  ctx.beginPath();
  ctx.roundRect(-17, -5, 34, 10, 3);
  ctx.fillStyle = destroyed ? COLOR.dim : enemy ? "#ff8a80" : "#7cc4ff";
  ctx.fill();
  ctx.strokeStyle = rgba("#000000", 0.6);
  ctx.lineWidth = 1.5;
  ctx.stroke();
  ctx.setLineDash([4, 3]);
  ctx.beginPath();
  ctx.moveTo(-12, 0);
  ctx.lineTo(12, 0);
  ctx.strokeStyle = rgba("#000000", 0.7);
  ctx.lineWidth = 1.25;
  ctx.stroke();
  ctx.restore();
}

/** Arrow-head marker for things that move. */
export function drawChevron(ctx: CanvasRenderingContext2D, x: number, y: number, angle: number, size: number, color: string, style: "solid" | "hollow" | "ghost") {
  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(angle);
  ctx.beginPath();
  ctx.moveTo(0, -size);
  ctx.lineTo(size * 0.72, size * 0.75);
  ctx.lineTo(0, size * 0.32);
  ctx.lineTo(-size * 0.72, size * 0.75);
  ctx.closePath();
  if (style === "solid") {
    ctx.fillStyle = color;
    ctx.fill();
    ctx.strokeStyle = rgba(COLOR.panel, 0.9);
    ctx.lineWidth = 1;
    ctx.stroke();
  } else {
    ctx.fillStyle = rgba(COLOR.panel, 0.75);
    ctx.fill();
    ctx.setLineDash(style === "ghost" ? [2, 3] : [4, 2]);
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.6;
    ctx.stroke();
  }
  ctx.restore();
}

/** Text with a dark halo so it reads on any map background. */
export function label(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, color = COLOR.ink, size = 11, align: CanvasTextAlign = "center") {
  ctx.font = `600 ${size}px ${FONT}`;
  ctx.textAlign = align;
  ctx.textBaseline = "top";
  ctx.lineJoin = "round";
  ctx.strokeStyle = rgba(COLOR.panel, 0.92);
  ctx.lineWidth = 3;
  ctx.strokeText(text, x, y);
  ctx.fillStyle = color;
  ctx.fillText(text, x, y);
}
