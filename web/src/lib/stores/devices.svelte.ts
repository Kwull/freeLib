import { api } from '../api/client';
import type { Device } from '../api/types';

// Devices come from the server in the user's own order (Settings → Devices, stored per user
// on the server); the first one is the default for quick sends here and for MCP clients.
export const devicesState = $state<{ items: Device[]; loaded: boolean }>({ items: [], loaded: false });

export async function loadDevices() {
  devicesState.items = await api.devices();
  devicesState.loaded = true;
}

/** The user's default device: the first in their order. */
export function defaultDevice(): Device | null {
  return devicesState.items[0] ?? null;
}

/** Kept for callers: the default device (first in the user's order). */
export function preferredDevice(): Device | null {
  return defaultDevice();
}

/** Saves a new order (all visible device ids, first = default). Optimistic. */
export async function reorderDevices(ids: number[]) {
  const byId = new Map(devicesState.items.map((d) => [d.id, d]));
  devicesState.items = ids.map((id) => byId.get(id)).filter((d): d is Device => !!d);
  devicesState.items = await api.orderDevices(ids);
}

/** The verb of a device's action: send (e-mail), download, export (server folder). */
export function deviceVerb(d: Device): 'send' | 'download' | 'export' {
  return d.kind === 'email' ? 'send' : d.kind === 'folder' ? 'export' : 'download';
}

/** The Apple Books device (seeded preset, else a download device named like it). */
export function appleBooksDevice(): Device | null {
  return devicesState.items.find((d) => d.preset === 'apple-books')
    ?? devicesState.items.find((d) => d.kind === 'download' && d.format === 'epub' && /apple|books|ibooks/i.test(d.name))
    ?? null;
}

/** Whether a device is a Kindle e-mail address (Send to Kindle). */
export function isKindleEmail(d: Device | null | undefined): boolean {
  return !!d && d.kind === 'email' && (d.preset === 'kindle-email' || /@(free\.)?kindle\.com$/i.test(d.target ?? ''));
}

/**
 * iPhone/iPad: downloads the book through a short-lived link, so Safari (also outside the
 * signed-in session, e.g. from a home-screen app) gets `application/epub+zip` as an
 * attachment and offers "Open in Books".
 */
export async function openInBooks(lib: number, bookId: number, device?: Device | null) {
  const link = await api.handoff({ library: lib, book: bookId, device: device?.id });
  location.assign(`${link.url}/file`);
}

/** "Kindle (USB) · AZW3" */
export function deviceCaption(d: Device): string {
  return `${d.name} · ${d.format === 'original' ? 'original' : d.format.toUpperCase()}`;
}
