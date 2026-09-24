import { api } from '../api/client';
import type { Device } from '../api/types';

export const devicesState = $state<{ items: Device[]; loaded: boolean }>({ items: [], loaded: false });

export async function loadDevices() {
  devicesState.items = await api.devices();
  devicesState.loaded = true;
}

export function defaultDevice(): Device | null {
  return devicesState.items[0] ?? null;
}
