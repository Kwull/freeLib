export type Toast = { id: number; text: string; kind: 'info' | 'error' };

export const toastState = $state<{ items: Toast[] }>({ items: [] });
let nextId = 1;

export function showToast(text: string, kind: Toast['kind'] = 'info') {
  const id = nextId++;
  toastState.items.push({ id, text, kind });
  setTimeout(() => {
    toastState.items = toastState.items.filter((t) => t.id !== id);
  }, 4000);
}
