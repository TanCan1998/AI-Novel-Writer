import { vi, type Mock } from 'vitest'

/**
 * `window.__TAURI_INTERNALS__` 测试桩（可复用 helper）。
 *
 * 真实运行时接缝（`@tauri-apps/api/core` / `event`）：
 * - `invoke(cmd, args, options)` → `window.__TAURI_INTERNALS__.invoke(cmd, args, options)`
 *   （**恒以 3 个实参**进入，第 3 个 `options` 通常为 `undefined`）；
 * - `listen(event, handler)` → `invoke('plugin:event|listen', { event, target, handler })`，
 *   其中 `handler` 是 `transformCallback(cb, once)` 返回的数值回调 id；
 * - Rust 侧 emit 时，登记的回调收到 `{ event, id, payload }`。
 *
 * 用法（node 测试，window 不存在——helper 会注入完整 window 桩）：
 * ```ts
 * let tauri: TauriInternalsHandle
 * beforeEach(() => {
 *   tauri = installTauriInternals({ commands: { db_blueprint_get_all: [] } })
 * })
 * afterEach(() => tauri.uninstall())
 * // 命令断言注意第 3 个实参 undefined：
 * expect(tauri.invoke).toHaveBeenCalledWith('db_blueprint_get_all', { expectedProjectPath: 'C:\\A' }, undefined)
 * ```
 *
 * 用法（browser 测试，window 已存在——helper 用 defineProperty 挂桩，保留其它属性）：
 * ```ts
 * tauri = installTauriInternals({ commands: { window_resolve_close: { success: true } } })
 * await act(async () => root.render(<TitleBar />))
 * tauri.emit('window:close-requested', { requestId: 'close-clean' })
 * ```
 */

/** Tauri 事件对象（Rust 侧 emit 时，transformCallback 登记的回调收到此结构）。 */
export interface TauriStubEvent<TPayload = unknown> {
  event: string
  id: number
  payload: TPayload
}

/** `plugin:event|listen` 登记的一条事件监听记录。 */
export interface TauriStubListenerRecord {
  /** `plugin:event|listen` 返回的事件 id（即 emit 时 `{event, id, payload}` 的 `id`）。 */
  eventId: number
  /** `transformCallback` 返回的回调 id（`plugin:event|unlisten` 用它定位回调）。 */
  callbackId: number
  /** 实际登记的回调函数（经 `ipc.on` 包装后的形态：收到事件对象，取 `event.payload` 转发）。 */
  callback: (event: TauriStubEvent) => void
}

export interface TauriInternalsStubOptions {
  /**
   * 命令名 → 返回值。值为函数时按 `(args) => result` 调用。
   * 命令名是 Tauri 侧命名（`fs:check-exists` → `fs_check_exists`）。
   */
  commands?: Record<string, unknown>
  /**
   * 未登记命令的兜底返回值。默认不设置——未登记命令抛错
   * （卫生策略：逼测试显式登记它实际调用的命令）。
   */
  fallback?: unknown
}

export interface TauriInternalsHandle {
  /** `__TAURI_INTERNALS__.invoke` 桩（断言目标；恒以 `(command, args, undefined)` 调用）。 */
  invoke: Mock
  /** `__TAURI_INTERNALS__.transformCallback` 桩：登记回调并返回数值 id。 */
  transformCallback: Mock
  /** `window.__TAURI_EVENT_PLUGIN_INTERNALS__`（真实 `_unlisten` 会先调它的 `unregisterListener`）。 */
  eventInternals: { unregisterListener: Mock }
  /** 当前已登记的事件监听（事件名 → 记录数组，只读视图）。 */
  listeners: ReadonlyMap<string, readonly TauriStubListenerRecord[]>
  /** 取某事件已登记的全部回调（手动逐回调触发的底层入口；一般用 `emit` 即可）。 */
  listenerCallbacks(event: string): readonly ((event: TauriStubEvent) => void)[]
  /** 模拟 Rust 侧 emit：向某事件的全部登记回调派发 `{ event, id, payload }`。 */
  emit(event: string, payload?: unknown): void
  /**
   * 卸载桩：node 环境删除 helper 注入的 `globalThis.window`；
   * browser 环境删除 `__TAURI_INTERNALS__` / `__TAURI_EVENT_PLUGIN_INTERNALS__` 两个属性。
   */
  uninstall(): void
}

/**
 * 安装 `window.__TAURI_INTERNALS__` 桩。
 *
 * - node 环境（`globalThis.window` 不存在）：`vi.stubGlobal('window', {...})` 注入仅含内部接缝的 window。
 * - browser 环境或已存在 window 桩：`Object.defineProperty` 挂载，保留 window 上其它属性。
 */
export function installTauriInternals(options: TauriInternalsStubOptions = {}): TauriInternalsHandle {
  const transformCallbacks = new Map<number, (event: TauriStubEvent) => void>()
  const listeners = new Map<string, TauriStubListenerRecord[]>()
  let nextCallbackId = 0
  let nextEventId = 0
  let usedStubGlobal = false

  const transformCallback: Mock = vi.fn((callback: (event: TauriStubEvent) => void) => {
    const id = ++nextCallbackId
    transformCallbacks.set(id, callback)
    return id
  })

  const eventInternals = { unregisterListener: vi.fn() }

  const invoke: Mock = vi.fn(async (command: string, args: Record<string, unknown> = {}) => {
    // 事件插件命令（`@tauri-apps/api/event` 的 listen/unlisten 走 invoke 通道）。
    if (command === 'plugin:event|listen') {
      const event = String(args.event)
      const callbackId = Number(args.handler)
      const callback = transformCallbacks.get(callbackId)
      if (!callback) {
        throw new Error(`[tauri-internals 桩] plugin:event|listen 引用了未登记的回调 id：${callbackId}`)
      }
      const record: TauriStubListenerRecord = { eventId: ++nextEventId, callbackId, callback }
      listeners.set(event, [...(listeners.get(event) ?? []), record])
      return record.eventId
    }
    if (command === 'plugin:event|unlisten') {
      const event = String(args.event)
      const eventId = Number(args.eventId)
      listeners.set(event, (listeners.get(event) ?? []).filter(record => record.eventId !== eventId))
      return undefined
    }

    const commands = options.commands ?? {}
    if (command in commands) {
      const result = commands[command]
      return typeof result === 'function'
        ? (result as (args: Record<string, unknown>) => unknown)(args)
        : result
    }
    if ('fallback' in options) return options.fallback
    throw new Error(`[tauri-internals 桩] 未登记的命令：${command}（请在 installTauriInternals({ commands }) 中登记）`)
  })

  const internals = { invoke, transformCallback }

  const existingWindow = (globalThis as { window?: unknown }).window
  if (existingWindow) {
    Object.defineProperty(existingWindow, '__TAURI_INTERNALS__', { configurable: true, value: internals })
    Object.defineProperty(existingWindow, '__TAURI_EVENT_PLUGIN_INTERNALS__', {
      configurable: true,
      value: eventInternals,
    })
  } else {
    vi.stubGlobal('window', {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: eventInternals,
    })
    usedStubGlobal = true
  }

  const emit = (event: string, payload?: unknown): void => {
    for (const record of listeners.get(event) ?? []) {
      record.callback({ event, id: record.eventId, payload })
    }
  }

  const uninstall = (): void => {
    if (usedStubGlobal) {
      Reflect.deleteProperty(globalThis, 'window')
      usedStubGlobal = false
      return
    }
    const windowObject = (globalThis as { window?: unknown }).window
    if (windowObject) {
      Reflect.deleteProperty(windowObject, '__TAURI_INTERNALS__')
      Reflect.deleteProperty(windowObject, '__TAURI_EVENT_PLUGIN_INTERNALS__')
    }
  }

  return {
    invoke,
    transformCallback,
    eventInternals,
    listeners,
    listenerCallbacks: (event: string) => (listeners.get(event) ?? []).map(record => record.callback),
    emit,
    uninstall,
  }
}
