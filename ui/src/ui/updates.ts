// New versions of the app itself. The desktop side asks GitHub for the
// latest release, downloads its installer, checks it against the published
// SHA-256 and starts it.

import { signal } from "@preact/signals";
import { isDesktop, shell } from "../transport";

export interface AppUpdate {
  current: string;
  latest: string;
  newer: boolean;
  /** Running from the portable ZIP: the new ZIP is unpacked over this folder. */
  portable: boolean;
  page: string;
  busy?: boolean;
  progress?: string;
  error?: string;
  checkedAt?: number;
}

export const appUpdate = signal<AppUpdate | null>(null);
export const updateMessage = signal("");

export async function checkForUpdate(manual = false) {
  if (!isDesktop) {
    if (manual) updateMessage.value = "Updates are handled by the desktop app.";
    return;
  }
  if (manual) updateMessage.value = "Checking…";
  try {
    const info = await shell<AppUpdate>("app_update_check");
    if (!info) return;
    appUpdate.value = { ...info, checkedAt: Date.now() };
    if (manual) updateMessage.value = info.newer ? `Version ${info.latest} is available.` : `You have the latest version (${info.current}).`;
  } catch (error) {
    if (manual) updateMessage.value = String((error as Error).message ?? error);
  }
}

export async function installUpdate() {
  const current = appUpdate.value;
  if (!current?.newer || current.busy) return;
  appUpdate.value = { ...current, busy: true, progress: "Downloading the update…", error: undefined };
  try {
    const result = await shell<string>("app_update_install");
    updateMessage.value = result ?? "";
    appUpdate.value = { ...appUpdate.value!, busy: false, progress: undefined };
  } catch (error) {
    const message = String((error as Error).message ?? error);
    updateMessage.value = message;
    appUpdate.value = { ...appUpdate.value!, busy: false, progress: undefined, error: message };
  }
}

/** Checks quietly a little after start, then every six hours. */
export function startUpdateChecks() {
  if (!isDesktop) return;
  setTimeout(() => checkForUpdate(false), 8000);
  setInterval(() => checkForUpdate(false), 6 * 3600 * 1000);
}
