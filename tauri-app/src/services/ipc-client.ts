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
 * - 未迁移频道（后续批次）由 `MIGRATED_CHANNELS` 前置拦截，抛出统一的
 *   「尚未迁移」提示，而不是 Tauri 原生的 `Unknown Error: Command xxx not found`。
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
import { MIGRATED_CHANNELS } from '../shared/migrated-channels'

/**
 * 频道 → invoke 命名参数名登记表（camelCase，与 Rust `#[tauri::command]`
 * 参数名的 camelCase 形式对应；Tauri 对每个参数 key 做 lowerCamelCase 归一，
 * 故 Rust 侧 `expected_project_path` / `_expected_project_path` 均对应
 * `expectedProjectPath`）。
 *
 * - 批次 A：config / window / skin / official-homepage / model-provider-resource
 * - 批次 B：fs（7）/ grant（3）/ project 带参频道（7）
 * - 批次 C：db 各子域带参频道
 * - 批次 D1：llm 模型管理带参频道（4）
 * - 后续批次迁移时在此追加登记；无参频道无需登记。
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
  'db:blueprint-commit-range': ['request', 'expectedProjectPath'],
  // 批次 C：蓝图角色同步（blueprints 子域 S2-c）
  'db:blueprint-character-sync-list-pending': ['expectedProjectPath'],
  'db:blueprint-character-sync-get': ['operationId', 'expectedProjectPath'],
  'db:blueprint-character-sync-complete': ['operationId', 'expectedProjectPath'],
  // 批次 C：草稿（drafts 子域 S3-a）
  'db:draft-create': ['params', 'expectedProjectPath'],
  'db:draft-list': ['chapterNumber', 'expectedProjectPath'],
  'db:draft-list-all': ['expectedProjectPath'],
  'db:draft-get-meta': ['id', 'expectedProjectPath'],
  'db:draft-get-full': ['id', 'expectedProjectPath'],
  'db:draft-get-latest': ['chapterNumber', 'expectedProjectPath'],
  'db:draft-get-finalized': ['chapterNumber', 'expectedProjectPath'],
  'db:draft-get-max-finalized-chapter': ['expectedProjectPath'],
  'db:draft-next-version': ['chapterNumber', 'expectedProjectPath'],
  'db:draft-update-status': ['id', 'status', 'wordCount', 'expectedProjectPath'],
  'db:draft-update-content': ['id', 'content', 'wordCount', 'expectedProjectPath'],
  'db:draft-delete': ['id', 'expectedProjectPath'],
  // 批次 C：修稿（revisions 子域 S3-b）
  'db:revision-create': ['params', 'expectedProjectPath'],
  'db:revision-replace-pending': ['params', 'expectedProjectPath'],
  'db:revision-list': ['baseDraftId', 'expectedProjectPath'],
  'db:revision-get-pending': ['baseDraftId', 'expectedProjectPath'],
  'db:revision-get-full': ['id', 'expectedProjectPath'],
  'db:revision-next-index': ['baseDraftId', 'expectedProjectPath'],
  'db:revision-merge': ['request', 'expectedProjectPath'],
  'db:revision-mark-merged': ['id', 'mergedToDraftId', 'expectedProjectPath'],
  'db:revision-mark-discarded': ['id', 'expectedProjectPath'],
  // 批次 C：审稿（reviews 子域 S3-c）
  'db:review-create': ['params', 'expectedProjectPath'],
  'db:review-list': ['baseDraftId', 'expectedProjectPath'],
  'db:review-get-latest': ['baseDraftId', 'expectedProjectPath'],
  'db:review-get-full': ['id', 'expectedProjectPath'],
  'db:review-next-index': ['baseDraftId', 'expectedProjectPath'],
  // 批次 C：后处理（post-process 子域 S3-d）
  'db:post-process-create-run': ['params', 'expectedProjectPath'],
  'db:post-process-get-latest-run': ['sourceType', 'sourceId', 'expectedProjectPath'],
  'db:post-process-get-steps': ['runId', 'expectedProjectPath'],
  'db:post-process-mark-step-ok': ['runId', 'stepKey', 'expectedProjectPath'],
  'db:post-process-mark-step-failed': ['runId', 'stepKey', 'errorMsg', 'expectedProjectPath'],
  'db:post-process-is-all-passed': ['sourceType', 'sourceId', 'expectedProjectPath'],
  // 批次 C：LLM 日志与摘要
  'db:log-llm-call': ['call', 'expectedProjectPath'],
  'db:get-llm-stats': ['expectedProjectPath'],
  'db:get-llm-history': ['limit', 'expectedProjectPath'],
  'db:save-summary-snapshot': ['chapterNumber', 'characterStates', 'expectedProjectPath'],
  'db:get-latest-summary': ['expectedProjectPath'],
  // 批次 C：项目生成数据清理
  'db:project-clear-generated-data': ['options', 'expectedProjectPath'],
  // 批次 D1：LLM 模型管理（无参频道：list-models / get-default-model /
  // get-default-embedding-model 无需登记）
  'llm:save-model': ['model'],
  'llm:delete-model': ['modelId'],
  'llm:set-default-model': ['modelId'],
  'llm:set-default-embedding-model': ['modelId'],
  // 批次 D2：LLM 生成执行（租约 2 频道）
  'llm:begin-execution-lease': ['modelId'],
  'llm:close-execution-lease': ['leaseId'],
  // 批次 D2-b：LLM 生成 / 流式 / 取消（3 频道）
  // 注：`llm:generate-stream` 的 `requestId` 由渲染层生成后传入，与 Rust
  // `llm_generate_stream(request_id, request)` 一一对应。
  'llm:generate': ['request'],
  'llm:generate-stream': ['requestId', 'request'],
  'llm:cancel': ['requestId'],
}

/** 频道 → Tauri 命令名（`channel:seg-name` → `channel_seg_name`）。 */
function toCommandName(channel: string): string {
  return channel.replace(/[:-]/g, '_')
}

/** 未迁移频道的统一错误文案（人工核对 `MIGRATED_CHANNELS` 时的定位入口）。 */
export function unMigratedChannelMessage(channel: string): string {
  return `[Tauri 适配] 频道 ${channel} 尚未迁移到 Tauri 侧（后续批次），已拒绝调用`
}

/**
 * Tauri 对未注册命令的报错格式（tauri 2.x：`Unknown Error: Command xxx not found`）。
 * 作为**兜底**：`MIGRATED_CHANNELS` 可能落后于 Rust 侧注册，此时仍翻译为友好文案。
 */
const TAURI_UNKNOWN_COMMAND_PATTERN = /Command\s+\S+\s+not found/u

/**
 * 调用后端命令：未迁移频道前置拦截；其余错误原样透传。
 *
 * 注意：本函数只负责「频道可用性」识别，不构造参数（参数由 `buildNamedArgs` 负责）。
 */
function invokeCommand<C extends InvokeChannel>(
  channel: C,
  named: Record<string, unknown>,
): Promise<AllInvokeChannels[C]['return']> {
  if (!MIGRATED_CHANNELS.has(channel)) {
    return Promise.reject(new Error(unMigratedChannelMessage(channel)))
  }
  return tauriInvoke(toCommandName(channel), named).catch((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error)
    if (TAURI_UNKNOWN_COMMAND_PATTERN.test(message)) {
      throw new Error(unMigratedChannelMessage(channel))
    }
    throw error
  }) as Promise<AllInvokeChannels[C]['return']>
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
  return invokeCommand(channel, named)
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
