// Mini map window: a heading-up scope around the player with the nearest
// air defences, enemy columns and strike alerts.

import { useEffect, useRef } from "preact/hooks";
import { isLive, siteName, snapshot, variant } from "./store";
import { COLOR, drawChevron, label, rgba } from "./map/symbols";
import { clock, km } from "./ui/format";

const RANGE_M = 40_000;

export function Overlay() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = canvasRef.current!;
    const ctx = canvas.getContext("2d")!;
    const draw = () => {
      const size = Math.min(canvas.clientWidth, canvas.clientHeight);
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      canvas.width = canvas.clientWidth * dpr;
      canvas.height = canvas.clientHeight * dpr;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, canvas.clientWidth, canvas.clientHeight);
      const cx = canvas.clientWidth / 2, cy = canvas.clientHeight / 2, radius = size / 2 - 6;
      ctx.beginPath();
      ctx.arc(cx, cy, radius, 0, Math.PI * 2);
      ctx.fillStyle = rgba(COLOR.panel, 0.72);
      ctx.fill();
      ctx.strokeStyle = rgba(COLOR.ink, 0.35);
      ctx.lineWidth = 1;
      ctx.stroke();
      for (const fraction of [0.25, 0.5, 0.75]) {
        ctx.beginPath();
        ctx.arc(cx, cy, radius * fraction, 0, Math.PI * 2);
        ctx.strokeStyle = rgba(COLOR.ink, 0.12);
        ctx.stroke();
      }
      const s = snapshot.value, v = variant.value;
      if (!s || !v || !isLive.value || !s.player) {
        label(ctx, s?.link === "offline" ? "GAME NOT DETECTED" : "WAITING FOR A BATTLE", cx, cy - 6, rgba(COLOR.ink, 0.7), 11);
        return;
      }
      const p = s.player;
      const heading = Math.atan2(p.dir[0], p.dir[1]);
      const scale = radius / RANGE_M;
      // world -> scope, nose up
      const place = (w: [number, number]): [number, number, number] => {
        const dx = w[0] - p.p[0], dz = w[1] - p.p[1];
        const d = Math.hypot(dx, dz), angle = Math.atan2(dx, dz) - heading;
        const r = Math.min(d, RANGE_M) * scale;
        return [cx + Math.sin(angle) * r, cy - Math.cos(angle) * r, d];
      };
      ctx.save();
      ctx.beginPath();
      ctx.arc(cx, cy, radius, 0, Math.PI * 2);
      ctx.clip();
      for (const cue of s.cues) {
        const [x, y, d] = place(cue.p);
        if (d < RANGE_M + cue.reach_m) {
          ctx.beginPath();
          ctx.arc(x, y, cue.reach_m * scale, 0, Math.PI * 2);
          ctx.fillStyle = rgba(COLOR.hostile, cue.inside ? 0.16 : 0.06);
          ctx.fill();
          ctx.strokeStyle = rgba(COLOR.hostile, 0.6);
          ctx.stroke();
        }
      }
      for (const alert of s.alerts) {
        const [x, y] = place(alert.p);
        ctx.beginPath();
        ctx.arc(x, y, Math.max(7, alert.r * scale), 0, Math.PI * 2);
        ctx.fillStyle = rgba(COLOR.amber, alert.phase === "now" ? 0.45 : 0.2);
        ctx.fill();
        ctx.strokeStyle = COLOR.amber;
        ctx.stroke();
      }
      ctx.restore();
      s.cues.forEach((cue, i) => {
        const [x, y, d] = place(cue.p);
        ctx.beginPath();
        ctx.moveTo(x, y - 5);
        ctx.lineTo(x + 5, y);
        ctx.lineTo(x, y + 5);
        ctx.lineTo(x - 5, y);
        ctx.closePath();
        ctx.fillStyle = d > RANGE_M ? rgba(COLOR.hostile, 0.5) : COLOR.hostile;
        ctx.fill();
        label(ctx, String(i + 1), x + 7, y - 6, COLOR.ink, 10, "left");
      });
      for (const convoy of s.convoys) {
        if (!convoy.enemy) continue;
        const [x, y, d] = place(convoy.p);
        if (d > RANGE_M) continue;
        drawChevron(ctx, x, y, Math.atan2(convoy.dir[0], convoy.dir[1]) - heading, 7, COLOR.amber, convoy.state === "observed" ? "solid" : "hollow");
      }
      ctx.beginPath();
      ctx.moveTo(cx, cy - 8);
      ctx.lineTo(cx + 6, cy + 6);
      ctx.lineTo(cx, cy + 2.5);
      ctx.lineTo(cx - 6, cy + 6);
      ctx.closePath();
      ctx.fillStyle = COLOR.ink;
      ctx.fill();
      label(ctx, `${RANGE_M / 1000} KM`, cx, cy - radius + 4, rgba(COLOR.ink, 0.55), 10);
    };
    const stop = snapshot.subscribe(draw);
    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    return () => {
      stop();
      observer.disconnect();
    };
  }, []);

  const s = snapshot.value, v = variant.value;
  const alert = s?.alerts[0];
  const threat = s?.cues.find((cue) => cue.inside);
  return (
    <div class="overlay" data-tauri-drag-region>
      <canvas ref={canvasRef} data-tauri-drag-region />
      <div class="overlay-lines" data-tauri-drag-region>
        {threat && v && <div class="line hostile">IN RANGE · {siteName(v.sites[threat.site])} · {km(threat.dist_m)}</div>}
        {alert && v && (
          <div class="line amber">
            {alert.phase === "now" ? "NOW" : `IN ${clock(alert.eta_s)}`} · {alert.count} units · {siteName(v.sites[alert.site])}
          </div>
        )}
      </div>
    </div>
  );
}
