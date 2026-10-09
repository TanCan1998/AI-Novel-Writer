// 浏览器测试配置（镜像基线 vitest.browser.config.ts，补充 @baseline 别名：
// tauri-app 渲染层经该别名引用 Electron 基线的纯类型契约）。
import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import { playwright } from '@vitest/browser-playwright'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { readFileSync } from 'node:fs'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const executablePath = process.env.AI_NOVEL_VITEST_CHROMIUM
const browserApiPort = Number(process.env.AI_NOVEL_VITEST_BROWSER_API_PORT || 63450)
const packageJson = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8'))

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@baseline': path.resolve(__dirname, '../electron'),
    },
  },
  optimizeDeps: {
    include: ['zustand/middleware'],
  },
  define: {
    __APP_VERSION__: JSON.stringify(packageJson.version),
  },
  test: {
    include: ['src/**/*.browser.tsx'],
    setupFiles: ['test/setup-locale.ts'],
    browser: {
      enabled: true,
      // 63315 is frequently reserved by Windows/HNS. Keep this overridable
      // for CI, but use an unreserved default for local browser regressions.
      api: { host: '127.0.0.1', port: browserApiPort },
      provider: playwright(executablePath ? { launchOptions: { executablePath } } : undefined),
      instances: [{ browser: 'chromium' }],
      headless: true,
      fileParallelism: false,
    },
  },
})
