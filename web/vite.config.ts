import { defineConfig, type Plugin } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

function mockApiPlugin(): Plugin {
  return {
    name: 'freelib-mock-api',
    apply: 'serve',
    async configureServer(server) {
      const { installMockApi } = await server.ssrLoadModule('/mock/server.ts') as { installMockApi: (s: typeof server.middlewares) => void };
      installMockApi(server.middlewares);
    },
  };
}

export default defineConfig(() => {
  const apiTarget = process.env.VITE_API;
  return {
    plugins: [svelte(), ...(apiTarget ? [] : [mockApiPlugin()])],
    server: {
      port: 5173,
      proxy: apiTarget
        ? {
            '/api': { target: apiTarget, changeOrigin: true },
            '/opds': { target: apiTarget, changeOrigin: true },
          }
        : undefined,
    },
    build: {
      target: 'es2020',
      sourcemap: true,
    },
  };
});
