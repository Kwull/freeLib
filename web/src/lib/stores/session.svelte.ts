import { api } from '../api/client';
import type { AuthMethods, Session, User } from '../api/types';

export const sessionState = $state<{ loaded: boolean; user: User | null; openMode: boolean; auth: AuthMethods }>({
  loaded: false,
  user: null,
  openMode: false,
  auth: { password: true, oidc: null },
});

export async function loadSession(): Promise<Session> {
  const s = await api.session();
  sessionState.user = s.user;
  sessionState.openMode = s.openMode;
  sessionState.auth = s.auth ?? { password: true, oidc: null };
  sessionState.loaded = true;
  return s;
}

export async function login(username: string, password: string) {
  const { user } = await api.login(username, password);
  sessionState.user = user;
}

export async function logout() {
  await api.logout();
  sessionState.user = null;
}

export function isLoggedIn(): boolean {
  return sessionState.openMode || sessionState.user !== null;
}
