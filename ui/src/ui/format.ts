export const km = (m: number) => (m >= 9950 ? `${Math.round(m / 1000)} km` : `${(m / 1000).toFixed(1)} km`);

export const clock = (s: number) => {
  const total = Math.max(0, Math.round(s));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
};

export const bearing = (deg: number) => `${String(Math.round((deg + 360) % 360)).padStart(3, "0")}°`;

/** "2 o'clock"-style direction relative to the nose. */
export const oclock = (relative: number) => {
  const hour = Math.round((((relative % 360) + 360) % 360) / 30) || 12;
  return `${hour} o'clock`;
};

export const br = (range: [number, number]) => `${range[0].toFixed(1)}–${range[1].toFixed(1)}`;

export function weaponLabel(name: string): string {
  if (!/^[0-9a-z_]+$/.test(name)) return name;
  return name
    .replace(/_(user_)?(cannon|rocket_launcher|launcher|naval)$/g, "")
    .replace(/_/g, " ")
    .replace(/^(\d+)mm /, "$1 mm ")
    .toUpperCase();
}
