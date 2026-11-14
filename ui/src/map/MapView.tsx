// The map. Two stacked canvases: the picture and grid (redrawn only when
// the camera moves) and everything live on top (redrawn when data changes).
// Nothing animates unless an alert is on screen, so an idle map costs no CPU.

import { useEffect, useRef } from "preact/hooks";
import { effect } from "@preact/signals";
import { packUrl } from "../transport";
import { convoysShown, core, enemyTeam, eraIndex, focusRequest, hoverAt, hovered, isLive, level, prefs, selected, siteName, siteShown, siteStates, snapshot, variant, view } from "../store";
import type { Level, Site } from "../types";
import { clearCover, coverCanvas, coverVersion, loadTerrain, requestCover, terrainReady } from "./cover";
import { COLOR, FONT, drawChevron, drawSite, isMinor, label, rgba, sideColor } from "./symbols";

interface Camera { x: number; z: number; ppm: number }

/** Darkened map picture plus smaller copies for sharp zoomed-out drawing. */
interface Artwork { id: string; src: string; mips: (ImageBitmap | HTMLCanvasElement)[] }

async function loadArtwork(l: Level): Promise<Artwork | null> {
  if (!l.image && !l.image_url) return null;
  const image = new Image();
  image.decoding = "async";
  image.crossOrigin = "anonymous";
  image.src = l.image_url ?? packUrl(`levels/${l.image}`);
  await image.decode();
  const mips: HTMLCanvasElement[] = [];
  let size = image.naturalWidth;
  let source: CanvasImageSource = image;
  for (let i = 0; i < 3 && size >= 512; i++) {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = size;
    const ctx = canvas.getContext("2d")!;
    ctx.imageSmoothingQuality = "high";
    // Toned down so symbols and rings stand out from the terrain.
    if (i === 0) ctx.filter = "brightness(0.6) saturate(0.75) contrast(1.04)";
    ctx.drawImage(source, 0, 0, size, size);
    mips.push(canvas);
    source = canvas;
    size = Math.floor(size / 2);
  }
  return { id: l.id, src: l.image_url ?? l.image ?? "", mips };
}

export function reachOf(unitId: string): number {
  const spec = core.value?.units[unitId];
  return spec ? spec.eng ?? spec.ai_eng ?? spec.gun ?? 0 : 0;
}

export function MapView({ onCursor }: { onCursor?: (world: [number, number] | null) => void }) {
  const wrap = useRef<HTMLDivElement>(null);
  const baseRef = useRef<HTMLCanvasElement>(null);
  const liveRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const host = wrap.current!, base = baseRef.current!, live = liveRef.current!;
    const bctx = base.getContext("2d", { alpha: false })!;
    const lctx = live.getContext("2d")!;
    const scratch = document.createElement("canvas");
    const sctx = scratch.getContext("2d")!;
    const cam: Camera = { x: 0, z: 0, ppm: 0.004 };
    let width = 1, height = 1, dpr = 1;
    let art: Artwork | null = null;
    let fitted = "", loadingArt = "";
    let baseDirty = true, liveDirty = true, frame = 0;
    let measure: { a: [number, number]; b: [number, number] } | null = null;
    let measuring = false;

    const sx = (wx: number) => (wx - cam.x) * cam.ppm + width / 2;
    const sy = (wz: number) => (cam.z - wz) * cam.ppm + height / 2;
    const toWorld = (px: number, py: number): [number, number] => [(px - width / 2) / cam.ppm + cam.x, cam.z - (py - height / 2) / cam.ppm];

    const fit = (l: Level) => {
      const [x0, z0, x1, z1] = l.bounds;
      cam.x = (x0 + x1) / 2;
      cam.z = (z0 + z1) / 2;
      cam.ppm = (Math.min(width, height) / Math.max(x1 - x0, z1 - z0)) * 2.4;
      // start on the part of the map where the action is
      const v = variant.value;
      if (v && v.sites.length) {
        const xs = v.sites.map((s) => s.p[0]), zs = v.sites.map((s) => s.p[1]);
        const [ax, bx, az, bz] = [Math.min(...xs), Math.max(...xs), Math.min(...zs), Math.max(...zs)];
        cam.x = (ax + bx) / 2;
        cam.z = (az + bz) / 2;
        cam.ppm = Math.min(width / (bx - ax + 12000), height / (bz - az + 12000));
      }
    };
    const clamp = () => {
      const l = level.value;
      if (!l) return;
      const span = Math.max(l.bounds[2] - l.bounds[0], l.bounds[3] - l.bounds[1]);
      cam.ppm = Math.min(1.2, Math.max((Math.min(width, height) / span) * 0.8, cam.ppm));
      cam.x = Math.min(l.bounds[2], Math.max(l.bounds[0], cam.x));
      cam.z = Math.min(l.bounds[3], Math.max(l.bounds[1], cam.z));
    };
    const invalidate = (all = false) => {
      if (all) baseDirty = true;
      liveDirty = true;
      if (!frame) frame = requestAnimationFrame(paint);
    };

    const resize = () => {
      const rect = host.getBoundingClientRect();
      width = Math.max(1, rect.width);
      height = Math.max(1, rect.height);
      dpr = Math.min(2, window.devicePixelRatio || 1);
      for (const canvas of [base, live]) {
        canvas.width = Math.round(width * dpr);
        canvas.height = Math.round(height * dpr);
      }
      invalidate(true);
    };

    // ------------------------------------------------------------ drawing
    const paintBase = () => {
      const l = level.value;
      bctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      bctx.fillStyle = "#101012";
      bctx.fillRect(0, 0, width, height);
      if (!l) return;
      const [x0, z0, x1, z1] = l.bounds;
      const left = sx(x0), top = sy(z1), w = (x1 - x0) * cam.ppm, h = (z1 - z0) * cam.ppm;
      if (art && art.id === l.id) {
        const scale = w / art.mips[0].width;
        const mip = scale < 0.3 && art.mips[2] ? art.mips[2] : scale < 0.6 && art.mips[1] ? art.mips[1] : art.mips[0];
        bctx.imageSmoothingQuality = "high";
        bctx.drawImage(mip, left, top, w, h);
      } else {
        bctx.fillStyle = "#14181a";
        bctx.fillRect(left, top, w, h);
      }
      if (prefs.value.layers.grid && l.grid_step) {
        const [gx, gz] = l.grid_step;
        bctx.strokeStyle = "rgba(233,236,238,0.10)";
        bctx.lineWidth = 1;
        bctx.beginPath();
        for (let x = x0, i = 0; x <= x1 + 1; x += gx, i++) {
          const px = Math.round(sx(x)) + 0.5;
          bctx.moveTo(px, top);
          bctx.lineTo(px, top + h);
        }
        for (let z = z1; z >= z0 - 1; z -= gz) {
          const py = Math.round(sy(z)) + 0.5;
          bctx.moveTo(left, py);
          bctx.lineTo(left + w, py);
        }
        bctx.stroke();
        if (gx * cam.ppm > 46) {
          bctx.font = `600 11px ${FONT}`;
          bctx.fillStyle = "rgba(242,242,247,0.3)";
          bctx.textBaseline = "top";
          for (let x = x0, i = 0; x < x1; x += gx, i++) {
            for (let z = z1, j = 0; z > z0; z -= gz, j++) {
              const px = sx(x), py = sy(z);
              if (px > -60 && px < width && py > -20 && py < height && j < 26) bctx.fillText(`${String.fromCharCode(65 + j)}${i + 1}`, px + 4, py + 3);
            }
          }
        }
      }
      bctx.strokeStyle = "rgba(233,236,238,0.25)";
      bctx.strokeRect(Math.round(left) + 0.5, Math.round(top) + 0.5, w, h);
    };

    const polyline = (ctx: CanvasRenderingContext2D, flat: number[]) => {
      ctx.beginPath();
      for (let i = 0; i + 1 < flat.length; i += 2) {
        if (i === 0) ctx.moveTo(sx(flat[i]), sy(flat[i + 1]));
        else ctx.lineTo(sx(flat[i]), sy(flat[i + 1]));
      }
    };

    const paintLive = (time: number) => {
      const ctx = lctx;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, width, height);
      const v = variant.value, c = core.value, l = level.value;
      if (!v || !c || !l) return false;
      const layers = prefs.value.layers;
      const bit = eraIndex.value;
      const enemy = enemyTeam.value;
      const states = siteStates.value;
      const snap = isLive.value ? snapshot.value : null;
      const onScreen = (x: number, y: number, pad = 40) => x > -pad && y > -pad && x < width + pad && y < height + pad;
      const detail = cam.ppm > 0.085; // individual vehicles become readable
      const showConvoys = convoysShown.value, enemyOnly = prefs.value.enemyOnly;
      let animating = false;

      // --- where air defences can actually see an aircraft, terrain included
      const cover = layers.cover && terrainReady.value === l.id ? coverCanvas() : null;
      if (cover && cover.width > 1) {
        const [bx0, bz0, bx1, bz1] = l.bounds;
        ctx.imageSmoothingEnabled = true;
        ctx.drawImage(cover, sx(bx0), sy(bz1), (bx1 - bx0) * cam.ppm, (bz1 - bz0) * cam.ppm);
      }

      // --- weapon reach of live air defences, merged into one envelope per side
      if (layers.rings) {
        const groups: Record<string, { x: number; y: number; r: number; minor: boolean; si: number }[]> = { enemy: [], friendly: [] };
        v.sites.forEach((site, si) => {
          const isEnemy = site.team === enemy;
          if (!siteShown(site)) return;
          if (site.moves && site.cat !== "ship") return;
          const state = states.get(si);
          let reach = 0, radar = 0;
          site.units.forEach((u, slot) => {
            if (!((u.e >> bit) & 1)) return;
            if (state?.units.get(slot)?.[0] === "destroyed") return;
            reach = Math.max(reach, reachOf(u.u), u.tr ?? 0);
            radar = Math.max(radar, c.units[u.u]?.det ?? 0);
          });
          const px = sx(site.p[0]), py = sy(site.p[1]);
          if (reach > 0) {
            const r = reach * cam.ppm;
            if (!(px + r < 0 || py + r < 0 || px - r > width || py - r > height)) {
              groups[isEnemy ? "enemy" : "friendly"].push({ x: px, y: py, r, minor: isMinor(site.cat), si });
            }
          }
          if (layers.radar && radar > 0 && isEnemy) {
            ctx.beginPath();
            ctx.arc(px, py, radar * cam.ppm, 0, Math.PI * 2);
            ctx.strokeStyle = rgba(COLOR.unknown, 0.2);
            ctx.lineWidth = 1;
            ctx.setLineDash([2, 6]);
            ctx.stroke();
            ctx.setLineDash([]);
          }
        });
        for (const key of ["friendly", "enemy"] as const) {
          const circles = groups[key];
          if (!circles.length) continue;
          const color = sideColor(key === "enemy");
          if (scratch.width !== live.width || scratch.height !== live.height) {
            scratch.width = live.width;
            scratch.height = live.height;
          }
          // Outline of the union: stroke every circle, then erase the insides.
          sctx.setTransform(dpr, 0, 0, dpr, 0, 0);
          sctx.globalCompositeOperation = "source-over";
          sctx.clearRect(0, 0, width, height);
          sctx.strokeStyle = color;
          sctx.lineWidth = 3;
          for (const circle of circles) {
            sctx.beginPath();
            sctx.arc(circle.x, circle.y, circle.r, 0, Math.PI * 2);
            sctx.stroke();
          }
          sctx.globalCompositeOperation = "destination-out";
          for (const circle of circles) {
            sctx.beginPath();
            sctx.arc(circle.x, circle.y, Math.max(0, circle.r - 0.75), 0, Math.PI * 2);
            sctx.fill();
          }
          // The covered area as one flat tint (overlaps do not add up).
          ctx.save();
          ctx.setTransform(1, 0, 0, 1, 0, 0);
          ctx.globalAlpha = key === "enemy" ? 0.8 : 0.4;
          ctx.drawImage(scratch, 0, 0);
          sctx.globalCompositeOperation = "source-over";
          sctx.clearRect(0, 0, width, height);
          sctx.fillStyle = color;
          for (const circle of circles) {
            sctx.beginPath();
            sctx.arc(circle.x, circle.y, circle.r, 0, Math.PI * 2);
            sctx.fill();
          }
          ctx.globalAlpha = key === "enemy" ? 0.09 : 0.035;
          if (!(cover && key === "enemy")) ctx.drawImage(scratch, 0, 0);
          ctx.restore();
          // The site under the cursor or selected shows its own ring.
          for (const circle of circles) {
            if (circle.si !== selected.value && circle.si !== hovered.value) continue;
            ctx.beginPath();
            ctx.arc(circle.x, circle.y, circle.r, 0, Math.PI * 2);
            ctx.strokeStyle = rgba(color, 0.9);
            ctx.lineWidth = 1.5;
            ctx.setLineDash(circle.minor ? [3, 4] : []);
            ctx.stroke();
            ctx.setLineDash([]);
          }
        }
      }

      // --- runways and ship routes
      for (const site of v.sites) {
        const color = sideColor(site.team === enemy);
        for (const rw of siteShown(site) ? site.rw ?? [] : []) {
          ctx.beginPath();
          ctx.moveTo(sx(rw[0]), sy(rw[1]));
          ctx.lineTo(sx(rw[2]), sy(rw[3]));
          ctx.strokeStyle = rgba(color, 0.85);
          ctx.lineWidth = Math.max(2, rw[4] * cam.ppm);
          ctx.stroke();
        }
        if (site.route && siteShown(site) && site.units.some((u) => (u.e >> bit) & 1)) {
          polyline(ctx, site.route.pts);
          if (site.route.closed) ctx.closePath();
          ctx.strokeStyle = rgba(color, 0.3);
          ctx.lineWidth = 1;
          ctx.setLineDash([6, 5]);
          ctx.stroke();
          ctx.setLineDash([]);
        }
      }

      // --- convoy roads ahead and uncertainty bands
      if (snap && layers.routes && showConvoys) {
        for (const convoy of snap.convoys) {
          if (!convoy.enemy && enemyOnly) continue;
          const color = sideColor(convoy.enemy);
          if (convoy.ahead.length >= 4) {
            polyline(ctx, convoy.ahead);
            ctx.strokeStyle = rgba(color, convoy.state === "scheduled" ? 0.25 : 0.5);
            ctx.lineWidth = 1.25;
            ctx.setLineDash([2, 4]);
            ctx.stroke();
            ctx.setLineDash([]);
          }
          if (convoy.band.length >= 4 && convoy.enemy) {
            polyline(ctx, convoy.band);
            ctx.strokeStyle = rgba(COLOR.amber, convoy.state === "scheduled" ? 0.2 : 0.38);
            ctx.lineWidth = 9;
            ctx.lineCap = "round";
            ctx.stroke();
            ctx.lineCap = "butt";
          }
        }
      }

      // --- where a strike would pay off
      if (snap) {
        for (const item of snap.opportunities) {
          const px = sx(item.p[0]), py = sy(item.p[1]), r = item.r * cam.ppm;
          if (!onScreen(px, py, r)) continue;
          ctx.beginPath();
          ctx.arc(px, py, Math.max(r, 10), 0, Math.PI * 2);
          ctx.strokeStyle = rgba(COLOR.amber, 0.5);
          ctx.lineWidth = 1;
          ctx.setLineDash([1, 4]);
          ctx.stroke();
          ctx.setLineDash([]);
        }
        for (const alert of snap.alerts) {
          const px = sx(alert.p[0]), py = sy(alert.p[1]), r = Math.max(alert.r * cam.ppm, 16);
          if (!onScreen(px, py, r)) continue;
          animating = true;
          // a slow breath, not a flash: noticeable without pulling the eye off the game
          const pulse = 0.5 + 0.5 * Math.sin(time / (alert.phase === "now" ? 420 : 900));
          const tone = alert.kind === "stack" ? COLOR.amber : COLOR.hostile;
          // big on screen = fainter, so a close-up is not drowned in colour
          const k = Math.min(1, 70 / r);
          const glow = ctx.createRadialGradient(px, py, 0, px, py, r * 1.6);
          glow.addColorStop(0, rgba(tone, ((alert.phase === "now" ? 0.34 : 0.16) + pulse * 0.12) * k));
          glow.addColorStop(0.62, rgba(tone, (0.1 + pulse * 0.06) * k));
          glow.addColorStop(1, rgba(tone, 0));
          ctx.fillStyle = glow;
          ctx.beginPath();
          ctx.arc(px, py, r * 1.6, 0, Math.PI * 2);
          ctx.fill();
          ctx.beginPath();
          ctx.arc(px, py, r, 0, Math.PI * 2);
          ctx.strokeStyle = rgba(tone, 0.75);
          ctx.lineWidth = 1.5;
          ctx.setLineDash(alert.phase === "now" ? [] : [5, 4]);
          ctx.stroke();
          ctx.setLineDash([]);
          const text = alert.phase === "now" ? `${alert.count} units` : `in ${Math.floor(alert.eta_s / 60)}:${String(Math.round(alert.eta_s % 60)).padStart(2, "0")}`;
          label(ctx, text, px, py - r - 15, tone, 11);
        }
      }

      // --- sites
      const order = v.sites.map((_, i) => i).sort((a, b) => Number(isMinor(v.sites[b].cat)) - Number(isMinor(v.sites[a].cat)));
      const labelZoom = cam.ppm > 0.012;
      for (const si of order) {
        const site = v.sites[si];
        if (site.cat === "radar_pad" || !siteShown(site)) continue;
        const state = states.get(si)!;
        if (state.total === 0 && site.cat !== "airfield") continue;
        const isEnemy = site.team === enemy;
        const px = sx(site.p[0]), py = sy(site.p[1]);
        if (!onScreen(px, py)) continue;
        const minor = isMinor(site.cat);
        const size = (minor ? 6.5 : 11) * (detail ? 0.85 : 1);

        if (detail) {
          // every vehicle of the site at its mission position
          site.units.forEach((u, slot) => {
            if (!((u.e >> bit) & 1)) return;
            const ux = sx(u.p[0]), uy = sy(u.p[1]);
            const unit = state.units.get(slot);
            const dead = unit?.[0] === "destroyed";
            ctx.beginPath();
            ctx.arc(ux, uy, 3.5, 0, Math.PI * 2);
            ctx.fillStyle = dead ? rgba(COLOR.panel, 0.8) : sideColor(isEnemy);
            ctx.fill();
            ctx.strokeStyle = dead ? rgba(COLOR.dim, 0.9) : unit?.[2] ? COLOR.ink : rgba(COLOR.panel, 0.9);
            ctx.lineWidth = 1.25;
            ctx.stroke();
            if (dead) {
              ctx.beginPath();
              ctx.moveTo(ux - 5, uy - 5);
              ctx.lineTo(ux + 5, uy + 5);
              ctx.moveTo(ux + 5, uy - 5);
              ctx.lineTo(ux - 5, uy + 5);
              ctx.strokeStyle = COLOR.hostile;
              ctx.lineWidth = 2;
              ctx.stroke();
            }
            if (cam.ppm > 0.3) label(ctx, c.units[u.u]?.s ?? c.units[u.u]?.n ?? u.u, ux, uy + 6, rgba(COLOR.ink, 0.85), 10);
          });
        }
        if (!detail || !minor) {
          drawSite(ctx, px, py - (detail ? 22 : 0), {
            enemy: isEnemy, cat: site.cat, size,
            strength: snap ? state.alive / Math.max(1, state.total) : null,
            total: state.total, destroyed: state.destroyed,
            selected: selected.value === si, hovered: hovered.value === si, seen: state.visible > 0,
          });
        }
        if (layers.labels && labelZoom && !minor && !detail) {
          label(ctx, state.destroyed ? `${siteName(site)} · destroyed` : siteName(site), px, py + size + 4, state.destroyed ? rgba(COLOR.hostile, 0.95) : rgba(COLOR.ink, 0.85), 11);
        }
      }

      // --- live contacts
      if (snap) {
        let aimNumber = 0;
        for (const contact of snap.contacts) {
          const px = sx(contact.p[0]), py = sy(contact.p[1]);
          if (contact.kind === "aim") {
            // the player's aiming points, numbered in the order the game lists them
            aimNumber += 1;
            if (!onScreen(px, py)) continue;
            ctx.beginPath();
            ctx.arc(px, py, 13, 0, Math.PI * 2);
            ctx.fillStyle = rgba(COLOR.panel, 0.55);
            ctx.fill();
            ctx.strokeStyle = COLOR.aim;
            ctx.lineWidth = 2.25;
            ctx.setLineDash([7, 4]);
            ctx.stroke();
            ctx.setLineDash([]);
            label(ctx, String(aimNumber), px, py - 7, COLOR.aim, 12.5);
            continue;
          }
          if (contact.kind === "poi") {
            // the player's point of interest: large, with a lead line from the aircraft
            if (snap.player) {
              ctx.beginPath();
              ctx.moveTo(sx(snap.player.p[0]), sy(snap.player.p[1]));
              ctx.lineTo(px, py);
              ctx.strokeStyle = rgba(COLOR.poi, 0.55);
              ctx.lineWidth = 1.25;
              ctx.setLineDash([3, 5]);
              ctx.stroke();
              ctx.setLineDash([]);
            }
            if (!onScreen(px, py)) continue;
            for (const [radius, width, alpha] of [[19, 5, 0.28], [19, 2.25, 1], [8, 2, 1]] as const) {
              ctx.beginPath();
              ctx.arc(px, py, radius, 0, Math.PI * 2);
              ctx.strokeStyle = rgba(COLOR.poi, alpha);
              ctx.lineWidth = width;
              ctx.stroke();
            }
            ctx.beginPath();
            for (const [dx, dy] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
              ctx.moveTo(px + dx * 12, py + dy * 12);
              ctx.lineTo(px + dx * 27, py + dy * 27);
            }
            ctx.strokeStyle = COLOR.poi;
            ctx.lineWidth = 2.25;
            ctx.stroke();
            const away = snap.player ? ` · ${(Math.hypot(contact.p[0] - snap.player.p[0], contact.p[1] - snap.player.p[1]) / 1000).toFixed(1)} km` : "";
            label(ctx, `Point of interest${away}`, px, py + 31, COLOR.poi, 12);
            continue;
          }
          if (contact.kind === "air" && !layers.aircraft) continue;
          if (contact.kind === "ground" && !showConvoys) continue;
          if (contact.aff !== "enemy" && enemyOnly) continue;
          if (!onScreen(px, py)) continue;
          const color = contact.aff === "enemy" ? COLOR.hostile : contact.aff === "ally" ? COLOR.friendly : COLOR.unknown;
          if (contact.kind === "air") {
            const angle = Math.atan2(contact.dir[0], contact.dir[1]);
            ctx.save();
            ctx.translate(px, py);
            ctx.rotate(angle);
            ctx.beginPath();
            ctx.moveTo(0, -6);
            ctx.lineTo(4.5, 5);
            ctx.lineTo(0, 2.5);
            ctx.lineTo(-4.5, 5);
            ctx.closePath();
            ctx.fillStyle = color;
            ctx.fill();
            ctx.restore();
          } else if (contact.kind === "ground") {
            ctx.fillStyle = color;
            ctx.fillRect(px - 2.5, py - 2.5, 5, 5);
          } else {
            ctx.beginPath();
            ctx.arc(px, py, 5, 0, Math.PI * 2);
            ctx.strokeStyle = color;
            ctx.lineWidth = 1.5;
            ctx.stroke();
          }
        }
        for (const convoy of showConvoys ? snap.convoys : []) {
          if (!convoy.enemy && enemyOnly) continue;
          const px = sx(convoy.p[0]), py = sy(convoy.p[1]);
          if (!onScreen(px, py)) continue;
          const color = sideColor(convoy.enemy);
          const angle = Math.atan2(convoy.dir[0], convoy.dir[1]);
          drawChevron(ctx, px, py, angle, 9, color, convoy.state === "observed" ? "solid" : convoy.state === "predicted" ? "hollow" : "ghost");
          if (convoy.enemy || cam.ppm > 0.02) {
            const tag = convoy.state === "observed" ? `×${convoy.n}` : convoy.state === "predicted" ? `×${convoy.n} est.` : "?";
            label(ctx, tag, px + 12, py - 6, convoy.state === "observed" ? COLOR.ink : COLOR.amber, 10.5, "left");
          }
        }
        if (snap.player) {
          const p = snap.player, px = sx(p.p[0]), py = sy(p.p[1]);
          const angle = Math.atan2(p.dir[0], p.dir[1]);
          ctx.beginPath();
          ctx.moveTo(px, py);
          ctx.lineTo(px + Math.sin(angle) * 10000 * cam.ppm, py - Math.cos(angle) * 10000 * cam.ppm);
          ctx.strokeStyle = rgba(COLOR.ink, 0.5);
          ctx.lineWidth = 1;
          ctx.stroke();
          ctx.save();
          ctx.translate(px, py);
          ctx.rotate(angle);
          ctx.beginPath();
          ctx.moveTo(0, -9);
          ctx.lineTo(7, 7);
          ctx.lineTo(0, 3);
          ctx.lineTo(-7, 7);
          ctx.closePath();
          ctx.fillStyle = COLOR.ink;
          ctx.fill();
          ctx.strokeStyle = COLOR.panel;
          ctx.lineWidth = 1.5;
          ctx.stroke();
          ctx.restore();
          label(ctx, "You", px + 12, py - 6, COLOR.ink, 11.5, "left");
        }
      }

      // --- ruler
      if (measure) {
        const [ax, az] = measure.a, [bx, bz] = measure.b;
        ctx.beginPath();
        ctx.moveTo(sx(ax), sy(az));
        ctx.lineTo(sx(bx), sy(bz));
        ctx.strokeStyle = COLOR.ink;
        ctx.lineWidth = 1.25;
        ctx.setLineDash([6, 4]);
        ctx.stroke();
        ctx.setLineDash([]);
        const d = Math.hypot(bx - ax, bz - az);
        const bearing = (Math.atan2(bx - ax, bz - az) * 180) / Math.PI;
        label(ctx, `${(d / 1000).toFixed(1)} km · ${String(Math.round((bearing + 360) % 360)).padStart(3, "0")}°`, sx(bx) + 10, sy(bz) + 8, COLOR.ink, 12, "left");
      }
      return animating;
    };

    const paint = (time: number) => {
      frame = 0;
      if (baseDirty) {
        baseDirty = false;
        paintBase();
      }
      if (liveDirty) {
        liveDirty = false;
        if (paintLive(time)) {
          // alerts breathe at a modest frame rate
          setTimeout(() => invalidate(), 50);
        }
      }
    };

    // -------------------------------------------------------- interaction
    const siteAt = (px: number, py: number): number | null => {
      const v = variant.value;
      if (!v) return null;
      const states = siteStates.value;
      let best: number | null = null, bestD = 18;
      v.sites.forEach((site: Site, si) => {
        if (site.cat === "radar_pad" || !siteShown(site)) return;
        if ((states.get(si)?.total ?? 0) === 0 && site.cat !== "airfield") return;
        const d = Math.hypot(sx(site.p[0]) - px, sy(site.p[1]) - py) + (isMinor(site.cat) ? 4 : 0);
        if (d < bestD) {
          bestD = d;
          best = si;
        }
      });
      return best;
    };

    let drag: { x: number; y: number; moved: boolean } | null = null;
    const point = (e: PointerEvent | WheelEvent): [number, number] => {
      const rect = host.getBoundingClientRect();
      return [e.clientX - rect.left, e.clientY - rect.top];
    };
    const down = (e: PointerEvent) => {
      const [px, py] = point(e);
      host.setPointerCapture(e.pointerId);
      if (e.shiftKey || e.button === 2) {
        measuring = true;
        const w = toWorld(px, py);
        measure = { a: w, b: w };
        return;
      }
      drag = { x: px, y: py, moved: false };
    };
    const move = (e: PointerEvent) => {
      const [px, py] = point(e);
      onCursor?.(toWorld(px, py));
      if (measuring && measure) {
        measure = { a: measure.a, b: toWorld(px, py) };
        invalidate();
        return;
      }
      if (drag) {
        const dx = px - drag.x, dy = py - drag.y;
        if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
        if (drag.moved && hovered.value !== null) {
          hovered.value = null;
          hoverAt.value = null;
        }
        cam.x -= dx / cam.ppm;
        cam.z += dy / cam.ppm;
        drag.x = px;
        drag.y = py;
        clamp();
        invalidate(true);
        return;
      }
      const hit = siteAt(px, py);
      if (hit !== hovered.value) hovered.value = hit;
      hoverAt.value = hit !== null ? { x: e.clientX, y: e.clientY } : null;
      host.style.cursor = hit !== null ? "pointer" : "grab";
    };
    const up = (e: PointerEvent) => {
      const [px, py] = point(e);
      if (measuring) {
        measuring = false;
        if (measure && Math.hypot(measure.a[0] - measure.b[0], measure.a[1] - measure.b[1]) * cam.ppm < 6) measure = null;
        invalidate();
        return;
      }
      if (drag && !drag.moved) selected.value = siteAt(px, py);
      drag = null;
    };
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      const [px, py] = point(e);
      const before = toWorld(px, py);
      cam.ppm *= Math.exp(-e.deltaY * 0.0015);
      clamp();
      const after = toWorld(px, py);
      cam.x += before[0] - after[0];
      cam.z += before[1] - after[1];
      clamp();
      invalidate(true);
    };
    const leave = () => {
      onCursor?.(null);
      if (hovered.value !== null) hovered.value = null;
      hoverAt.value = null;
    };
    const noMenu = (e: Event) => e.preventDefault();

    host.addEventListener("pointerdown", down);
    host.addEventListener("pointermove", move);
    host.addEventListener("pointerup", up);
    host.addEventListener("pointerleave", leave);
    host.addEventListener("wheel", wheel, { passive: false });
    host.addEventListener("contextmenu", noMenu);
    const observer = new ResizeObserver(resize);
    observer.observe(host);
    resize();

    const stops = [
      effect(() => {
        const l = level.value;
        variant.value;
        if (l && fitted !== `${l.id}:${view.value.variant}`) {
          fitted = `${l.id}:${view.value.variant}`;
          fit(l);
          clamp();
        }
        if (l && (art?.id !== l.id || art.src !== (l.image_url ?? l.image ?? "")) && loadingArt !== `${l.id}:${l.image_url ?? l.image}`) {
          {
            loadingArt = `${l.id}:${l.image_url ?? l.image}`;
            if (art?.id !== l.id) art = null;
            loadArtwork(l).then((a) => {
              if (level.value?.id === l.id) {
                art = a;
                invalidate(true);
              }
            }).catch(() => {});
          }
        }
        invalidate(true);
      }),
      effect(() => {
        snapshot.value; siteStates.value; selected.value; hovered.value; eraIndex.value; enemyTeam.value;
        invalidate();
      }),
      effect(() => {
        prefs.value.layers; prefs.value.show; prefs.value.enemyOnly;
        invalidate(true);
      }),
      effect(() => {
        coverVersion.value;
        invalidate();
      }),
      effect(() => {
        // Recompute terrain cover when the sites, their state or the altitude change.
        const l = level.value, v = variant.value, c = core.value, p = prefs.value;
        if (!p.layers.cover) {
          clearCover();
          return;
        }
        if (!l || !v || !c || !l.terrain) return;
        if (terrainReady.value !== l.id) {
          loadTerrain(l).catch(() => {});
          return;
        }
        const bit = eraIndex.value, enemy = enemyTeam.value, states = siteStates.value;
        const sites: { x: number; z: number; reach: number }[] = [];
        v.sites.forEach((site, si) => {
          if (site.team !== enemy || (site.moves && site.cat !== "ship")) return;
          const state = states.get(si);
          let reach = 0;
          site.units.forEach((u, slot) => {
            if (((u.e >> bit) & 1) && state?.units.get(slot)?.[0] !== "destroyed") reach = Math.max(reach, reachOf(u.u), u.tr ?? 0);
          });
          if (reach > 0) sites.push({ x: site.p[0], z: site.p[1], reach });
        });
        const liveAlt = isLive.value ? snapshot.value?.player?.alt_m : null;
        const follow = p.coverLive && liveAlt != null;
        requestCover(l, sites, follow ? liveAlt! : p.coverAlt, follow ? false : p.coverAgl);
      }),
      effect(() => {
        const request = focusRequest.value;
        if (!request) return;
        cam.x = request.p[0];
        cam.z = request.p[1];
        if (request.zoom) cam.ppm = Math.max(cam.ppm, request.zoom);
        clamp();
        invalidate(true);
      }),
    ];
    return () => {
      stops.forEach((stop) => stop());
      observer.disconnect();
      host.removeEventListener("pointerdown", down);
      host.removeEventListener("pointermove", move);
      host.removeEventListener("pointerup", up);
      host.removeEventListener("pointerleave", leave);
      host.removeEventListener("wheel", wheel);
      host.removeEventListener("contextmenu", noMenu);
      if (frame) cancelAnimationFrame(frame);
    };
  }, []);

  return (
    <div class="map" ref={wrap}>
      <canvas ref={baseRef} />
      <canvas ref={liveRef} />
    </div>
  );
}
