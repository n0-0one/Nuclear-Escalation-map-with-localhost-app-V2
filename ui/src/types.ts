// Shapes of the data pack and of the engine's snapshot. Kept loose on
// purpose: a newer data pack or engine may add fields.

export type V2 = [number, number];

export interface Era { id: string; label: string; rank: [number, number]; br: [number, number] }
export interface Weapon { n: string; ammo?: number; min?: number; max?: number; g?: string; rl?: number }
export interface UnitSpec {
  n: string; s?: string; icon: string; type: string; model: string;
  det?: number; trk?: number; eng?: number; ai_eng?: number; gun?: number; kph?: number; w?: Weapon[];
}
export interface LevelRef {
  id: string; level: string; name: string; bounds: [number, number, number, number]; file: string;
  has_image: boolean; has_terrain: boolean; has_roads: boolean; variants: string[];
}
export interface Core {
  schema: number; game_version: string; eras: Era[]; units: Record<string, UnitSpec>; unit_art: string[];
  nukes: { id: string; yield_kt: number }[]; levels: LevelRef[];
  rules: Record<string, any>; respawn: any[]; template_restore: { restore_seconds?: number; depot_repair_seconds?: number };
  convoys: Record<string, any[]>;
}
export interface Placement { u: string; p: V2; e: number; r?: string; l?: string; tr?: number; x?: number }
export interface Site {
  id: string; cls: string; team: number; cat: string; p: V2; alt: number; units: Placement[]; bld?: Placement[];
  rw?: [number, number, number, number, number][]; route?: { pts: number[]; closed: boolean; speed_kph: number };
  moves?: string; cap?: number;
}
export interface Variant {
  id: string; mission: string; mirror: boolean; sites: Site[];
  supply_links: [string, string, number][]; armor_links: [string, string, number][];
  mobile: { id: string; team: number; kind: string; spawn: [number, number, number] | null; move: [number, number, number][] }[];
  bombers: { id: string; team: number; p: V2; alt: number }[];
  escalation: number[]; score_limit?: number; time_limit_min?: number;
}
export interface Level {
  id: string; level: string; name: string; bounds: [number, number, number, number]; water: number;
  grid_step?: [number, number]; variants: Variant[]; image?: string; image_url?: string; roads?: string;
  terrain?: { image: string; w: number; h: number; min_m: number; step_m: number; water: number };
}
export interface Manifest { game_version: string; generated: string; datamine: { repository: string; commit: string }; pending_game_changes?: string[] }

export type Conf = "none" | "guess" | "likely" | "confirmed" | "manual";
export type Status = "unknown" | "alive" | "destroyed";
export type Proof = "none" | "likely" | "confirmed" | "manual";

export interface Settings {
  colors_swapped: boolean; strike_radius_m: number; strike_min_units: number; alert_lead_s: number;
  manual_level: string | null; manual_variant: string | null; manual_team: number | null; manual_era: string | null;
}
export interface Player { p: V2; dir: V2; alt_m: number | null; tas_kmh: number | null; vehicle: string | null; vehicle_name: string | null; br: number | null; grid: string | null }
export interface Contact { p: V2; dir: V2; aff: "ally" | "enemy" | "unknown"; kind: "air" | "ground" | "zone"; icon: string; blink: boolean }
export interface Convoy {
  id: number; team: number; enemy: boolean; n: number; p: V2; dir: V2; speed: number;
  state: "observed" | "predicted" | "scheduled"; kind: "supply" | "armor" | null; from: number | null; to: number | null;
  band: number[]; ahead: number[]; eta_s: number | null; unseen_s: number | null;
}
export interface Alert { id: string; kind: "stack" | "attack"; phase: "now" | "soon"; site: number; convoy: number; p: V2; r: number; count: number; eta_s: number; uncertain: boolean }
export interface Opportunity { p: V2; r: number; count: number; friendly: number; site: number | null; convoys: number[] }
export interface Cue { site: number; p: V2; dist_m: number; bearing: number; relative: number | null; reach_m: number; inside: boolean; alive: number; total: number; grid: string | null }
export interface LogEntry { at: number; battle_t: number | null; kind: "kill" | "lost" | "info"; text: string; site: number | null }
export interface Battle {
  level: string | null; variant: string | null; team: number | null; era: string | null;
  level_conf: Conf; side_conf: Conf; era_conf: Conf; era_options: string[]; why: string[];
  clock_s: number | null; unlocated_kills: number; mission_status: string;
}
export interface Snapshot {
  seq: number; link: "offline" | "menu" | "battle"; unknown_map: boolean; bounds: [number, number, number, number] | null;
  battle: Battle; player: Player | null; contacts: Contact[];
  units: [number, number, Status, Proof, boolean, number | null][];
  convoys: Convoy[]; alerts: Alert[]; opportunities: Opportunity[]; cues: Cue[];
  schedules: { site: number; next_in: [number, number]; locked: boolean }[];
  log: LogEntry[]; settings: Settings; demo: { speed: number; t: number } | null; recording: string | null;
}
