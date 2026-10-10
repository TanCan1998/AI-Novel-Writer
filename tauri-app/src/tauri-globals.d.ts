/**
 * 迁移期全局声明（Tauri 版）。
 *
 * Tauri 版**不注入 preload 桥接**：渲染层一律经 `src/services/ipc-client.ts`
 * 用 `@tauri-apps/api/core` 的 `invoke` 访问后端；`__TAURI_INTERNALS__` 由
 * Tauri 运行时注入（见 `isTauri()` 判定）。
 *
 * 这里只为「尚未现代化的旧测试桩」保留可选的 `window` 属性类型，使合并上游后
 * 仍以 `window.aiNovelAPI` / `window.velaAPI` 书写的既有测试能通过类型检查；
 * 业务代码与新增测试不得依赖这两个属性（后续「测试桩现代化」批次会整体替换为
 * `__TAURI_INTERNALS__` 桩）。
 */
interface Window {
  aiNovelAPI?: import('./shared/desktop-api').AiNovelAPI
  velaAPI?: import('./shared/desktop-api').AiNovelAPI
}
