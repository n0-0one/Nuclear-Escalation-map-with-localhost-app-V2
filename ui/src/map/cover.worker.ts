// Terrain-masked air-defence coverage, computed off the main thread.
//
// For every live enemy air-defence site, rays are walked outward over the
// height grid. A cell counts as covered when an aircraft at the chosen
// altitude there is inside the site's weapon reach and no terrain between
// the two rises above the line of sight. Earth curvature and each missile's
// altitude limits are not modelled.

interface Terrain { size: number; cell: number; ground: Float32Array }
interface Emitter { x: number; y: number; reach: number }

const ANTENNA_M = 10;
let terrain: Terrain | null = null;

self.onmessage = (event: MessageEvent) => {
  const message = event.data;
  if (message.type === "terrain") {
    terrain = { size: message.size, cell: message.cell, ground: message.ground };
    return;
  }
  if (message.type !== "compute" || !terrain) return;
  const { size, cell, ground } = terrain;
  const emitters: Emitter[] = message.emitters;
  const altitude: number = message.altitude;
  const aboveGround: boolean = message.aboveGround;
  // 0 = clear, 1 = inside reach but hidden by terrain, 2 = exposed
  const cover = new Uint8Array(size * size);
  const targetAt = (index: number) => (aboveGround ? ground[index] + altitude : altitude);

  for (const emitter of emitters) {
    const ox = Math.min(size - 1, Math.max(0, Math.floor(emitter.x)));
    const oy = Math.min(size - 1, Math.max(0, Math.floor(emitter.y)));
    const eye = ground[oy * size + ox] + ANTENNA_M;
    const reach2 = emitter.reach * emitter.reach;
    const corner = Math.max(
      Math.hypot(emitter.x, emitter.y), Math.hypot(size - emitter.x, emitter.y),
      Math.hypot(emitter.x, size - emitter.y), Math.hypot(size - emitter.x, size - emitter.y),
    );
    const cells = Math.min(emitter.reach / cell, corner);
    const steps = Math.ceil(cells);
    // enough rays that neighbours are under a cell apart at the far end
    const rays = Math.max(64, Math.ceil(Math.PI * 2 * cells * 1.5));
    cover[oy * size + ox] = 2;
    for (let ray = 0; ray < rays; ray++) {
      const angle = (ray / rays) * Math.PI * 2;
      const dx = Math.cos(angle), dy = Math.sin(angle);
      let horizon = -Infinity;
      for (let step = 1; step <= steps; step++) {
        const cx = Math.floor(emitter.x + dx * step), cy = Math.floor(emitter.y + dy * step);
        if (cx < 0 || cy < 0 || cx >= size || cy >= size) break;
        const index = cy * size + cx;
        const distance = step * cell;
        const surface = ground[index];
        const target = targetAt(index);
        const rise = target - eye;
        if (distance * distance + rise * rise <= reach2) {
          const visible = target >= surface && rise / distance >= horizon - 1e-6;
          if (visible) cover[index] = 2;
          else if (cover[index] === 0) cover[index] = 1;
        }
        const slope = (surface - eye) / distance;
        if (slope > horizon) horizon = slope;
      }
    }
  }
  let exposed = 0, hidden = 0;
  for (let i = 0; i < cover.length; i++) {
    if (cover[i] === 2) exposed++;
    else if (cover[i] === 1) hidden++;
  }
  (self as unknown as Worker).postMessage({ type: "cover", id: message.id, cover, size, exposed, hidden }, [cover.buffer]);
};
