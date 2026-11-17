import { signal } from "@preact/signals";
import { command, isDesktop, shell } from "../transport";
import { applySettings, core, manifest, prefs, setPref, snapshot, view } from "../store";
import { settingsOpen } from "./Bar";
import { Icon } from "./shared";
import { appUpdate, checkForUpdate, installUpdate, updateMessage } from "./updates";

const message = signal("");
const diagText = signal<string | null>(null);
interface AppInfo { version: string; pack_source: string; portable?: boolean; program_folder?: string; data_folder?: string }
const appInfo = signal<AppInfo | null>(null);

function Slider(props: { label: string; value: number; min: number; max: number; step: number; unit: string; onChange: (n: number) => void }) {
  return (
    <label class="field">
      <span>{props.label}</span>
      <input type="range" min={props.min} max={props.max} step={props.step} value={props.value} onChange={(e) => props.onChange(Number((e.target as HTMLInputElement).value))} />
      <output>{props.value} {props.unit}</output>
    </label>
  );
}

function Toggle(props: { label: string; hint?: string; on: boolean; onClick: () => void }) {
  return (
    <button class="switch-row" onClick={props.onClick} aria-pressed={props.on}>
      <span>
        <b>{props.label}</b>
        {props.hint && <small>{props.hint}</small>}
      </span>
      <i class={`switch ${props.on ? "on" : ""}`} />
    </button>
  );
}

/** A plain-text summary of what the game reported, to paste into a bug report. */
function buildDiagnostics(): string {
  const s = snapshot.value, c = core.value, m = manifest.value;
  if (!s) return "No data yet.";
  const report = {
    app: appInfo.value?.version ?? "browser",
    data: { game_version: c?.game_version, built: m?.generated, source: appInfo.value?.pack_source },
    link: s.link,
    demo: !!s.demo,
    unknown_map: s.unknown_map,
    bounds: s.bounds,
    battle: s.battle,
    player: s.player && { vehicle: s.player.vehicle, br: s.player.br, alt_m: s.player.alt_m, grid: s.player.grid },
    counts: { contacts: s.contacts.length, units_with_state: s.units.length, convoys: s.convoys.length, alerts: s.alerts.length, cues: s.cues.length },
    game_reported: s.diag,
    settings: s.settings,
  };
  return JSON.stringify(report, null, 1);
}

async function copyDiagnostics() {
  const text = buildDiagnostics();
  diagText.value = text;
  try {
    await navigator.clipboard.writeText(text);
    message.value = "Copied. Paste it wherever you report the problem.";
  } catch {
    message.value = "Select the text below and copy it.";
  }
}

export function SettingsDialog() {
  if (!settingsOpen.value) return null;
  const s = snapshot.value, c = core.value, m = manifest.value, v = view.value;
  if (!s || !c) return null;
  const settings = s.settings;
  if (isDesktop && !appInfo.value) shell<AppInfo>("app_info").then((info) => (appInfo.value = info)).catch(() => {});
  const close = () => {
    settingsOpen.value = false;
    diagText.value = null;
    message.value = "";
  };
  const startDemo = () => command("demo_start", { level: v.level, variant: v.variant, era: v.era, team: v.team, speed: 1 }).then(close);
  const checkUpdate = async () => {
    message.value = "Checking…";
    try {
      message.value = (await shell<string>("update_data")) ?? "Updates are handled by the desktop app.";
    } catch (e) {
      message.value = String((e as Error).message ?? e);
    }
  };
  const yields = [...new Set(c.nukes.map((n) => n.yield_kt))].map((kt) => `${kt} kt`).join(", ");
  return (
    <div class="scrim" onClick={(e) => e.target === e.currentTarget && close()}>
      <div class="dialog material" role="dialog" aria-label="Settings">
        <header>
          <h2>Settings</h2>
          <button class="tool plain" onClick={close} aria-label="Close">{Icon.close}</button>
        </header>
        <div class="dialog-body">
          <h3>Strike alerts</h3>
          <div class="group">
            <Slider label="Blast radius to plan with" value={settings.strike_radius_m} min={300} max={5000} step={100} unit="m" onChange={(n) => applySettings({ strike_radius_m: n })} />
            <Slider label="Smallest group worth showing" value={settings.strike_min_units} min={3} max={30} step={1} unit="units" onChange={(n) => applySettings({ strike_min_units: n })} />
            <Slider label="Warn before a convoy arrives" value={settings.alert_lead_s} min={30} max={300} step={10} unit="s" onChange={(n) => applySettings({ alert_lead_s: n })} />
            <Toggle label="Sound" hint="A soft tone when a convoy reaches an enemy site" on={prefs.value.sound} onClick={() => setPref({ sound: !prefs.value.sound })} />
          </div>
          <p class="hint">The game files give each bomb's yield ({yields}) but not its blast radius, so set the radius you see in game.</p>

          <h3>Game</h3>
          <div class="group">
            <Toggle label="Swapped team colours" hint="Turn on if your game shows enemies in blue and friends in red" on={settings.colors_swapped} onClick={() => applySettings({ colors_swapped: !settings.colors_swapped })} />
          </div>
          <div class="buttons">
            {s.recording ? (
              <button onClick={() => command("record", { on: false })}>Stop recording</button>
            ) : (
              <button onClick={() => command("record", { on: true }).catch((e) => (message.value = String(e.message)))} disabled={!!s.demo}>Record this battle</button>
            )}
            <button onClick={() => command("reset_battle")}>Start over on this battle</button>
            <button onClick={copyDiagnostics}>Copy diagnostics</button>
          </div>
          {s.recording && <p class="hint path">{s.recording}</p>}
          {diagText.value && <textarea class="diag" readOnly value={diagText.value} onFocus={(e) => (e.target as HTMLTextAreaElement).select()} />}
          <p class="hint">The app only reads the page War Thunder itself serves on this computer (127.0.0.1:8111). It never touches the game's memory or files and sends nothing about you to the internet.</p>

          <h3>Demo battle</h3>
          <div class="buttons">
            {s.demo ? (
              <>
                <button onClick={() => command("demo_stop")}>Stop demo</button>
                {[1, 4, 10].map((speed) => (
                  <button key={speed} class={s.demo?.speed === speed ? "on" : ""} onClick={() => command("demo_speed", { speed })}>{speed}×</button>
                ))}
              </>
            ) : (
              <button onClick={startDemo}>Start a demo battle</button>
            )}
          </div>
          <p class="hint">A simulated battle on the map chosen at the top left, built from the real mission data.</p>

          {isDesktop && (
            <>
              <h3>Mini map window</h3>
              <div class="buttons">
                <button onClick={() => shell("overlay", { action: "show" })}>Show</button>
                <button onClick={() => shell("overlay", { action: "hide" })}>Hide</button>
                <button onClick={() => shell("overlay", { action: "locked" })}>Click-through</button>
                <button onClick={() => shell("overlay", { action: "unlocked" })}>Movable</button>
              </div>
              <p class="hint">A small always-on-top window with nearby threats and alerts. Run the game in borderless or windowed mode to see it over the game.</p>
            </>
          )}

          <h3>App</h3>
          <div class="group">
            <div class="row-info">
              <span>
                <b>Nuclear Thunder Live {appInfo.value?.version ?? appUpdate.value?.current ?? ""}</b>
                <small>
                  {appUpdate.value?.newer
                    ? `Version ${appUpdate.value.latest} is available.`
                    : appUpdate.value
                      ? "You have the latest version."
                      : isDesktop ? "Checks for new versions by itself." : "Running in a browser."}
                </small>
              </span>
              {appUpdate.value?.newer ? (
                <button class="primary" disabled={appUpdate.value.busy} onClick={installUpdate}>
                  {appUpdate.value.busy ? appUpdate.value.progress ?? "Working…" : "Update now"}
                </button>
              ) : (
                isDesktop && <button onClick={() => checkForUpdate(true)}>Check now</button>
              )}
            </div>
          </div>
          {updateMessage.value && <p class="hint strong">{updateMessage.value}</p>}
          {appUpdate.value?.newer && (
            <p class="hint">
              {appUpdate.value.portable
                ? "The new version is unpacked into this app's folder and started. Your settings and recordings are kept."
                : "The installer opens and the app closes. Your settings are kept."}
            </p>
          )}

          {isDesktop && appInfo.value?.data_folder && (
            <>
              <h3>Folders</h3>
              <div class="group">
                <div class="row-info">
                  <span>
                    <b>Program</b>
                    <small class="path">{appInfo.value.program_folder}</small>
                  </span>
                  <button onClick={() => shell("open_folder", { which: "program" })}>Open</button>
                </div>
                <div class="row-info">
                  <span>
                    <b>Settings, map updates and recordings</b>
                    <small class="path">{appInfo.value.data_folder}</small>
                  </span>
                  <button onClick={() => shell("open_folder", { which: "data" })}>Open</button>
                </div>
              </div>
              <p class="hint">
                {appInfo.value.portable
                  ? "Portable copy: everything this app uses lives in its own folder. Move or delete the folder and that is all."
                  : "Want everything in one folder? Use the portable version instead of the installer."}
              </p>
            </>
          )}

          <h3>Map data</h3>
          <dl class="facts">
            <dt>Game version</dt><dd>{c.game_version}</dd>
            <dt>Built</dt><dd>{m?.generated?.slice(0, 10) ?? "—"}</dd>
            {appInfo.value && (<><dt>App version</dt><dd>{appInfo.value.version}</dd></>)}
          </dl>
          {(m?.pending_game_changes?.length ?? 0) > 0 && <p class="hint">The game's event files changed after the weapon ranges in this data were extracted. Unit positions are current; ranges may lag until the next update.</p>}
          <div class="buttons">
            {isDesktop && <button onClick={checkUpdate}>Check for new map data</button>}
          </div>
          {message.value && <p class="hint strong">{message.value}</p>}
          <p class="hint">Unit positions, battle ratings and names come from the public War Thunder datamine. Weapon ranges, terrain and roads come from majorlier's Nuclear Thunder Interactive Map. Not affiliated with Gaijin Entertainment.</p>
        </div>
      </div>
    </div>
  );
}
