// Owns the terrain grid and the coverage picture drawn on the map.

import { signal } from "@preact/signals";
import { packUrl } from "../transport";
import type { Level } from "../types";

const GRID = 512;

/** Bumped whenever a new coverage picture is ready. */
export const coverVersion = signal(0);
export const coverStats = signal<{ exposed: number; hidden: number; altitude: number } | null>(null);
export const terrainReady = signal<string | null>(null);

let worker: Worker | null = null;
let canvas: HTMLCanvasElement | null = null;
let terrainFor = "";
let ground: Float32Array | null = null;
let requestId = 0;
let lastKey = "";
let pendingAltitude = 0;

export const coverCanvas = () => canvas;

/** Height of the ground (or water surface) at a world position, in metres. */
export function groundAt(level: Level, x: number, z: number): number | null {
  if (!ground || terrainFor !== level.id) return null;
  const [x0, z0, x1, z1] = level.bounds;
  const gx = Math.floor(((x - x0) / (x1 - x0)) * GRID), gy = Math.floor(((z1 - z) / (z1 - z0)) * GRID);
  if (gx < 0 || gy < 0 || gx >= GRID || gy >= GRID) return null;
  return ground[gy * GRID + gx];
}

function ensureWorker(): Worker {
  if (worker) return worker;
  worker = new Worker(new URL("./cover.worker.ts", import.meta.url), { type: "module" });
  worker.onmessage = (event: MessageEvent) => {
    const message = event.data;
    if (message.type !== "cover" || message.id !== requestId) return;
    canvas ??= document.createElement("canvas");
    canvas.width = canvas.height = message.size;
    const ctx = canvas.getContext("2d")!;
    const image = ctx.createImageData(message.size, message.size);
    const cover: Uint8Array = message.cover;
    for (let i = 0; i < cover.length; i++) {
      const o = i * 4;
      if (cover[i] === 2) {
        image.data[o] = 255; image.data[o + 1] = 90; image.data[o + 2] = 71; image.data[o + 3] = 74;
      } else if (cover[i] === 1) {
        // inside reach but behind a hill: a usable corridor
        image.data[o] = 127; image.data[o + 1] = 209; image.data[o + 2] = 139; image.data[o + 3] = 42;
      }
    }
    ctx.putImageData(image, 0, 0);
    coverStats.value = { exposed: message.exposed, hidden: message.hidden, altitude: pendingAltitude };
    coverVersion.value++;
  };
  return worker;
}

/** Loads the level's height picture and hands a coarse grid to the worker. */
export async function loadTerrain(level: Level): Promise<void> {
  if (terrainFor === level.id || !level.terrain) return;
  terrainFor = level.id;
  ground = null;
  terrainReady.value = null;
  lastKey = "";
  const t = level.terrain;
  const response = await fetch(packUrl(`levels/${t.image}`));
  // Heights are packed into the red and green channels; they must not be colour-managed.
  const bitmap = await createImageBitmap(await response.blob(), { colorSpaceConversion: "none", premultiplyAlpha: "none" });
  const source = document.createElement("canvas");
  source.width = bitmap.width;
  source.height = bitmap.height;
  const ctx = source.getContext("2d", { willReadFrequently: true })!;
  ctx.drawImage(bitmap, 0, 0);
  const pixels = ctx.getImageData(0, 0, bitmap.width, bitmap.height).data;
  const grid = new Float32Array(GRID * GRID);
  const water = t.water ?? level.water ?? 0;
  const sx = bitmap.width / GRID, sy = bitmap.height / GRID;
  for (let y = 0; y < GRID; y++) {
    for (let x = 0; x < GRID; x++) {
      // highest point of the block, so a ridge is never averaged away
      let top = -Infinity;
      const x0 = Math.floor(x * sx), x1 = Math.max(x0 + 1, Math.floor((x + 1) * sx));
      const y0 = Math.floor(y * sy), y1 = Math.max(y0 + 1, Math.floor((y + 1) * sy));
      for (let py = y0; py < y1; py++) {
        for (let px = x0; px < x1; px++) {
          const o = (py * bitmap.width + px) * 4;
          const h = t.min_m + (pixels[o] * 256 + pixels[o + 1]) * t.step_m;
          if (h > top) top = h;
        }
      }
      grid[y * GRID + x] = Math.max(top, water);
    }
  }
  if (terrainFor !== level.id) return;
  ground = grid;
  const cell = (level.bounds[2] - level.bounds[0]) / GRID;
  ensureWorker().postMessage({ type: "terrain", size: GRID, cell, ground: grid.slice() });
  terrainReady.value = level.id;
}

export interface CoverSite { x: number; z: number; reach: number }

export function requestCover(level: Level, sites: CoverSite[], altitude: number, aboveGround: boolean) {
  if (terrainReady.value !== level.id) return;
  const [x0, , x1, z1] = level.bounds;
  const scale = GRID / (x1 - x0);
  const emitters = sites.map((s) => ({ x: (s.x - x0) * scale, y: (z1 - s.z) * scale, reach: s.reach }));
  const key = `${level.id}|${Math.round(altitude / 100)}|${aboveGround}|${emitters.map((e) => `${e.x.toFixed(1)},${e.y.toFixed(1)},${e.reach}`).join(";")}`;
  if (key === lastKey) return;
  lastKey = key;
  pendingAltitude = altitude;
  ensureWorker().postMessage({ type: "compute", id: ++requestId, emitters, altitude, aboveGround });
}

export function clearCover() {
  lastKey = "";
  if (canvas) {
    canvas.getContext("2d")!.clearRect(0, 0, canvas.width, canvas.height);
    coverVersion.value++;
  }
  coverStats.value = null;
}
