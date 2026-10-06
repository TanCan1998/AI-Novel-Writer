import { defineConfig } from 'vite';
import { configDefaults } from 'vitest/config';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { readFileSync } from 'node:fs';

// ESM 中不存在 __dirname，需要用 import.meta.url 来模拟
const __dirname = path.dirname(fileURLToPath(import.meta.url));

// 从 package.json 读取版本号，构建时注入到前端
const pkg = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf-8'));

// https://vitejs.dev/config/
export default defineConfig({
  // Tauri WebView 通过自定义协议加载产物，保持资源路径相对化。
  base: './',
  plugins: [tailwindcss(), react()],
  resolve: {
    alias: {
      // 类型层面引用 Electron 基线的共享契约（electron/repositories|services 的纯类型），
      // 仅 import type，运行时零依赖；上游同步后契约类型自动跟随，无需双份维护。
      '@baseline': path.resolve(__dirname, '../electron'),
    },
  },
  publicDir: 'public',
  server: {
    // 与 Electron 基线（5180）错开，支持两栈并行运行做行为对照。
    host: '127.0.0.1',
    port: 5190,
    strictPort: true,
    watch: {
      ignored: ['**/docs/**'],
    },
  },
  define: {
    // 构建时将 package.json 版本注入为全局常量，避免 StatusBar 硬编码版本号
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  optimizeDeps: {
    entries: ['index.html', 'src/**/*.{ts,tsx}'],
  },
  test: {
    // Test copy must not follow the operating-system locale of a CI runner.
    setupFiles: ['test/setup-locale.ts'],
    exclude: [...configDefaults.exclude, '**/dist/**'],
  },
  build: {
    rollupOptions: {
      onwarn(warning, defaultHandler) {
        // 过滤掉已知的无害警告
        if (warning.code === 'INEFFECTIVE_DYNAMIC_IMPORT') return;
        if (warning.message?.includes('Invalid key')) return;
        defaultHandler(warning);
      },
    },
  },
});
