import { api } from '../api/client';
import { getPref, setPref } from './prefs.svelte';
import type { Device } from '../api/types';

export const devicesState = $state<{ items: Device[]; loaded: boolean }>({ items: [], loaded: false });

export async function loadDevices() {
  devicesState.items = await api.devices();
  devicesState.loaded = true;
}

export function defaultDevice(): Device | null {
  return devicesState.items[0] ?? null;
}

/** The device of the last send (per user prefs), else the first one. */
export function preferredDevice(): Device | null {
  const last = getPref<number | null>('lastDevice', null);
  return devicesState.items.find((d) => d.id === last) ?? defaultDevice();
}

export function rememberDevice(id: number) {
  if (getPref('lastDevice', null) !== id) setPref('lastDevice', id);
}
