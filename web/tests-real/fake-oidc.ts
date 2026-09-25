// A tiny OpenID Connect provider for the real-server single sign-on spec: discovery, JWKS
// (an RSA key generated at start), an authorize endpoint that approves at once, a token
// endpoint that checks PKCE and issues RS256 ID tokens, and userinfo.
import http from 'node:http';
import { AddressInfo } from 'node:net';
import { createHash, generateKeyPairSync, randomBytes, sign } from 'node:crypto';

export type FakeUser = { sub: string; preferred_username?: string; email?: string; groups?: string[] };

export type FakeOidc = {
  issuer: string;
  user: FakeUser;
  /** When set, /authorize redirects back with this OAuth error instead of a code. */
  error: string | null;
  close: () => Promise<void>;
};

const b64u = (b: Buffer | string) => Buffer.from(b).toString('base64url');

export async function startFakeOidc(clientId: string): Promise<FakeOidc> {
  const { privateKey, publicKey } = generateKeyPairSync('rsa', { modulusLength: 2048 });
  const jwk = { ...(publicKey.export({ format: 'jwk' }) as Record<string, string>), kid: 'k1', use: 'sig', alg: 'RS256' };
  const codes = new Map<string, { nonce: string; challenge: string; redirect: string }>();
  const tokens = new Map<string, FakeUser>();
  const state: FakeOidc = { issuer: '', user: { sub: 'sub-1' }, error: null, close: async () => {} };

  const json = (res: http.ServerResponse, status: number, body: unknown) => {
    res.writeHead(status, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(body));
  };

  const server = http.createServer((req, res) => {
    const url = new URL(req.url ?? '/', state.issuer);
    if (url.pathname === '/.well-known/openid-configuration') {
      return json(res, 200, {
        issuer: state.issuer,
        authorization_endpoint: `${state.issuer}/authorize`,
        token_endpoint: `${state.issuer}/token`,
        jwks_uri: `${state.issuer}/jwks`,
        userinfo_endpoint: `${state.issuer}/userinfo`,
        response_types_supported: ['code'],
        subject_types_supported: ['public'],
        id_token_signing_alg_values_supported: ['RS256'],
        code_challenge_methods_supported: ['S256'],
      });
    }
    if (url.pathname === '/jwks') return json(res, 200, { keys: [jwk] });
    if (url.pathname === '/authorize') {
      const q = url.searchParams;
      const redirect = q.get('redirect_uri') ?? '';
      const back = new URL(redirect);
      back.searchParams.set('state', q.get('state') ?? '');
      if (state.error) {
        back.searchParams.set('error', state.error);
      } else {
        const code = b64u(randomBytes(16));
        codes.set(code, { nonce: q.get('nonce') ?? '', challenge: q.get('code_challenge') ?? '', redirect });
        back.searchParams.set('code', code);
      }
      res.writeHead(302, { Location: back.toString() });
      return res.end();
    }
    if (url.pathname === '/token' && req.method === 'POST') {
      let body = '';
      req.on('data', (c) => (body += c));
      req.on('end', () => {
        const p = new URLSearchParams(body);
        const issued = codes.get(p.get('code') ?? '');
        codes.delete(p.get('code') ?? '');
        if (!issued) return json(res, 400, { error: 'invalid_grant' });
        if (b64u(createHash('sha256').update(p.get('code_verifier') ?? '').digest()) !== issued.challenge) {
          return json(res, 400, { error: 'invalid_grant', error_description: 'PKCE' });
        }
        if (p.get('redirect_uri') !== issued.redirect) return json(res, 400, { error: 'invalid_grant' });
        const now = Math.floor(Date.now() / 1000);
        const access = b64u(randomBytes(24));
        tokens.set(access, state.user);
        const claims = { iss: state.issuer, aud: clientId, iat: now, exp: now + 300, nonce: issued.nonce, ...state.user };
        const input = `${b64u(JSON.stringify({ alg: 'RS256', typ: 'JWT', kid: 'k1' }))}.${b64u(JSON.stringify(claims))}`;
        const sig = sign('sha256', Buffer.from(input), privateKey);
        json(res, 200, { access_token: access, token_type: 'Bearer', expires_in: 300, id_token: `${input}.${b64u(sig)}` });
      });
      return;
    }
    if (url.pathname === '/userinfo') {
      const u = tokens.get((req.headers.authorization ?? '').replace(/^Bearer /, ''));
      return u ? json(res, 200, u) : json(res, 401, { error: 'invalid_token' });
    }
    res.writeHead(404);
    res.end();
  });
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  state.issuer = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  state.close = () => new Promise<void>((r) => server.close(() => r()));
  return state;
}
