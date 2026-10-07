/**
 * 渲染进程的 IPC 客户端 — 类型安全的后端通信封装（Tauri 底层）
 *
 * 用法：
 *   import { ipc } from '@/services/ipc-client'
 *   const result = await ipc.invoke('project:create', { name: '...' })
 *
 * 迁移说明（docs/handoffs/2026-10-06-tauri-migration-status.md）：
 * - 原 Electron 底层为 preload 注入的 `window.velaAPI.invoke(channel, ...args)`
 *   （可变位置参数）；Tauri 2 的 `invoke(cmd, args)` 使用命名参数对象。
 * - 频道 → 命令名映射为机械规则：`config:get` → `config_get`、
 *   `official-homepage:open` → `official_homepage_open`（`:`/`-` → `_`）。
 * - 位置参数 → 命名参数依赖 `CHANNEL_ARG_NAMES` 登记表（频道 → Rust 命令
 *   参数名 camelCase 序列，与 `src/shared/ipc-channels.ts` 的 args 位置一一对应）。
 *   批次迁移时在此登记；未登记的非空参频道立即抛错，防止静默错配。
 * - 项目域频道沿用「尾部注入 projectSession」约定：Tauri 下作为命名参数
 *   `projectSession` 传入，对应 Rust 命令尾参 `project_session`。
 */
import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { listen as tauriListen, once as tauriOnce } from '@tauri-apps/api/event'
import type {
  AllInvokeChannels,
  AllEventChannels,
  InvokeChannel,
  EventChannel,
} from '../shared/ipc-channels'
import type { ProjectSessionContext } from '../shared/ipc-channels'
import { getActiveProjectSessionContext } from '../shared/project-session-context'

/**
 * 频道 → invoke 命名参数名登记表（camelCase，与 Rust `#[tauri::command]`
 * 参数名的 camelCase 形式对应；Tauri 对每个参数 key 做 lowerCamelCase 归一，
 * 故 Rust 侧 `expected_project_path` / `_expected_project_path` 均对应
 * `expectedProjectPath`）。
 *
 * - 批次 A：config / window / skin / official-homepage / model-provider-resource
 * - 批次 B：fs（7）/ grant（3）/ project 带参频道（7）
 * - 批次 B+ 迁移时在此追加登记；无参频道无需登记。
 */
const CHANNEL_ARG_NAMES: Record<string, readonly string[]> = {
  'config:set': ['config'],
  'skin:execute': ['command'],
  'window:resolve-close': ['requestId', 'decision'],
  'model-provider-resource:open': ['resource'],
  // 批次 B：项目文件系统（fs 基础 7 频道）
  'fs:read-file': ['filePath', 'expectedProjectPath'],
  'fs:write-file': ['filePath', 'content', 'expectedProjectPath'],
  'fs:list-dir': ['dirPath', 'expectedProjectPath'],
  'fs:mkdir': ['dirPath', 'expectedProjectPath'],
  'fs:check-exists': ['filePath', 'expectedProjectPath'],
  'fs:read-json': ['filePath', 'expectedProjectPath'],
  'fs:write-json': ['filePath', 'data', 'expectedProjectPath'],
  // 批次 B：外部文件授权（grant 3 频道）
  'fs:grant-read-file': ['grantId', 'relativePath'],
  'fs:grant-write-file': ['grantId', 'relativePath', 'content'],
  'fs:grant-mkdir': ['grantId', 'relativePath'],
  // 批次 B：项目生命周期（带参频道）
  'project:create': ['config', 'requestToken', 'rendererProjectPath'],
  'project:open': ['projectPath', 'requestToken', 'rendererProjectPath'],
  'project:save': ['projectId', 'data', 'expectedProjectPath'],
  'project:update-config': ['projectId', 'data', 'expectedProjectPath'],
  'project:recent-remove': ['projectPath'],
  'project:delete': ['projectPath', 'projectId', 'sessionLease'],
  'project:smoke-open-confirm': ['projectPath'],
  // 批次 C：项目数据库（project_core 子域 4 频道）
  'db:close': ['expectedProjectPath'],
  'db:project-core-get': ['expectedProjectPath'],
  'db:project-core-update': ['data', 'expectedProjectPath'],
  'db:project-core-synopsis-commit': ['request', 'expectedProjectPath'],
  'db:character-get-all': ['expectedProjectPath'],
  'db:character-roster-read': ['expectedProjectPath'],
  'db:character-roster-commit': ['request', 'expectedProjectPath'],
  // 批次 C：章节蓝图（blueprints 子域 S2-a）
  'db:blueprint-get-all': ['expectedProjectPath'],
  'db:blueprint-get': ['chapterNumber', 'expectedProjectPath'],
  'db:blueprint-upsert': ['data', 'expectedProjectPath'],
  'db:blueprint-upsert-many': ['items', 'expectedProjectPath'],
  'db:blueprint-update-notes': ['chapterNumber', 'notes', 'expectedProjectPath'],
  'db:blueprint-delete': ['chapterNumber', 'expectedProjectPath'],
  'db:blueprint-clear-all': ['expectedProjectPath'],
}

/** 频道 → Tauri 命令名（`channel:seg-name` → `channel_seg_name`）。 */
function toCommandName(channel: string): string {
  return channel.replace(/[:-]/g, '_')
}

/** 是否运行在 Tauri 环境（由 Tauri 注入的内部全局判定）。 */
function isTauri(): boolean {
  return typeof window !== 'undefined'
    && !!(window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
}

/**
 * 把契约的定位参数数组转成 Tauri invoke 的命名参数对象，
 * 并按会话租约约定（ADR 0001）附加 `projectSession` 尾参。
 */
function buildNamedArgs(
  channel: string,
  args: readonly unknown[],
  projectSession?: ProjectSessionContext,
): Record<string, unknown> {
  const named: Record<string, unknown> = {}
  if (args.length > 0) {
    const names = CHANNEL_ARG_NAMES[channel]
    if (!names) {
      throw new Error(`[Tauri 适配] 频道 ${channel} 尚未迁移（参数名未登记），已拒绝调用`)
    }
    if (args.length > names.length) {
      throw new Error(`[Tauri 适配] 频道 ${channel} 传参 ${args.length} 个，超过契约登记的 ${names.length} 个`)
    }
    args.forEach((value, index) => { named[names[index]] = value })
  }
  if (projectSession) {
    named.projectSession = projectSession
  }
  return named
}

/**
 * 这些请求的授权来自用户选择后由后端签发的 grant，或固定 app-data 边界；
 * 它们绝不能借用、也不需要当前项目会话。
 */
function isCapabilityOrAppDataChannel(channel: string): boolean {
  return channel.startsWith('fs:grant-')
    || channel.startsWith('dialog:select-')
    || channel.startsWith('prompt:')
    || channel.startsWith('skills:')
}

function isProjectScopedChannel(channel: string): boolean {
  if (isCapabilityOrAppDataChannel(channel)) return false
  return channel.startsWith('db:')
    || channel.startsWith('kb:')
    || channel.startsWith('chapter:')
    || channel.startsWith('fs:')
    || channel === 'project:save'
    || channel === 'project:update-config'
    || channel === 'project:delete'
}

function invokeWithSession<C extends InvokeChannel>(
  channel: C,
  args: AllInvokeChannels[C]['args'],
  context?: ProjectSessionContext,
): Promise<AllInvokeChannels[C]['return']> {
  let projectSession: ProjectSessionContext | undefined
  if (isProjectScopedChannel(channel)) {
    projectSession = context ?? getActiveProjectSessionContext() ?? undefined
    if (!projectSession) {
      throw new Error('缺少当前项目会话，已拒绝项目数据访问')
    }
  }
  const named = buildNamedArgs(channel, args, projectSession)
  return tauriInvoke(toCommandName(channel), named) as Promise<AllInvokeChannels[C]['return']>
}

/** 类型安全的 IPC 客户端 */
export const ipc = {
  /**
   * 调用后端命令并等待返回值（类型安全）
   *
   * @example
   * const result = await ipc.invoke('project:create', { name: '我的小说', path: '/path', genre: '玄幻', targetAudience: '男频' })
   */
  invoke: async <C extends InvokeChannel>(
    channel: C,
    ...args: AllInvokeChannels[C]['args']
  ): Promise<AllInvokeChannels[C]['return']> => {
    console.log('[ipc-client.invoke] 调用通道:', channel, '参数数量:', args.length)
    const result = await invokeWithSession(channel, args)
    console.log('[ipc-client.invoke] 调用完成:', channel)
    return result
  },

  /** 工作流/工具在启动处冻结会话后，必须使用此入口而不是重新读取 currentProject。 */
  invokeWithProjectSession: async <C extends InvokeChannel>(
    context: ProjectSessionContext,
    channel: C,
    ...args: AllInvokeChannels[C]['args']
  ): Promise<AllInvokeChannels[C]['return']> => {
    if (!isProjectScopedChannel(channel)) {
      throw new Error(`通道不属于项目会话范围：${channel}`)
    }
    return invokeWithSession(channel, args, context)
  },

  /**
   * 监听后端推送的事件（返回取消订阅函数）
   *
   * @example
   * const unsub = ipc.on('llm:stream-chunk', (data) => console.log(data.chunk))
   * // 组件卸载时取消
   * unsub()
   */
  on: <C extends EventChannel>(
    channel: C,
    callback: (data: AllEventChannels[C]) => void,
  ): (() => void) => {
    const unlisten = tauriListen(channel, (event) => {
      callback(event.payload as AllEventChannels[C])
    })
    // Tauri listen 是异步注册；返回同步取消函数，与 Electron 版语义对齐
    let cancelled = false
    let actualUnlisten: (() => void) | null = null
    unlisten.then((fn) => {
      if (cancelled) fn()
      else actualUnlisten = fn
    })
    return () => {
      if (actualUnlisten) actualUnlisten()
      else cancelled = true
    }
  },

  /** 一次性监听 */
  once: <C extends EventChannel>(
    channel: C,
    callback: (data: AllEventChannels[C]) => void,
  ) => {
    tauriOnce(channel, (event) => {
      callback(event.payload as AllEventChannels[C])
    }).catch((error) => {
      console.error('[ipc-client.once] 事件监听注册失败:', channel, error)
    })
  },

  /** 单向发送（无返回值，fire-and-forget） */
  send: (channel: string, ...args: unknown[]) => {
    tauriInvoke(toCommandName(channel), buildNamedArgs(channel, args))
      .catch((error) => {
        console.error('[ipc-client.send] 单向发送失败:', channel, error)
      })
  },

  /** 是否有可用的 IPC 后端（Tauri 2；保留原属性名以对齐基线调用面） */
  get isElectron(): boolean {
    return isTauri()
  },

  /** 设置窗口缩放级别（阶段 3 迁移项：Tauri webview zoom 绑定待接） */
  setZoomLevel: (level: number) => {
    void level
    console.warn('[ipc-client] setZoomLevel 尚未在 Tauri 侧实现（阶段 3 迁移项）')
  },

  /** 设置绝对缩放比例（阶段 3 迁移项） */
  setZoomFactor: (factor: number) => {
    void factor
    console.warn('[ipc-client] setZoomFactor 尚未在 Tauri 侧实现（阶段 3 迁移项）')
  },

  /** 获取当前缩放级别（阶段 3 迁移项） */
  getZoomLevel: () => {
    console.warn('[ipc-client] getZoomLevel 尚未在 Tauri 侧实现（阶段 3 迁移项）')
    return 0
  }
}
