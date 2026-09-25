import type { Job, Library } from './types';

export type EventHandlers = {
  onJob?: (job: Job) => void;
  onLibrary?: (lib: Library) => void;
  onOpen?: () => void;
};

/** Subscribes to GET /api/v1/events (SSE) with automatic reconnect + backoff. */
export function connectEvents(handlers: EventHandlers): () => void {
  let es: EventSource | null = null;
  let closed = false;
  let attempt = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  function open() {
    if (closed) return;
    es = new EventSource('/api/v1/events', { withCredentials: true });
    es.addEventListener('open', () => {
      attempt = 0;
      handlers.onOpen?.();
    });
    es.addEventListener('job', (e) => {
      try { handlers.onJob?.(JSON.parse((e as MessageEvent).data)); } catch { /* ignore */ }
    });
    es.addEventListener('library', (e) => {
      try { handlers.onLibrary?.(JSON.parse((e as MessageEvent).data)); } catch { /* ignore */ }
    });
    es.addEventListener('error', () => {
      es?.close();
      if (closed) return;
      attempt += 1;
      const delay = Math.min(1000 * 2 ** attempt, 15000);
      timer = setTimeout(open, delay);
    });
  }

  open();

  return () => {
    closed = true;
    if (timer) clearTimeout(timer);
    es?.close();
  };
}
