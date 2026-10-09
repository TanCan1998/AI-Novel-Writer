import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import {
  setActiveProjectSessionContext,
} from '../../shared/project-session-context'
import { ipc } from '../ipc-client'

// Tauri 语义：ipc-client 直接调 `@tauri-apps/api/core` 的 invoke（命名参数），
// 不再有 Electron `window.velaAPI` 读取点。这里注入 Tauri 运行时的内部全局桩
// （`window.__TAURI_INTERNALS__.invoke`），让真实 ipc-client 逻辑（频道映射 /
// 命名参数 / 会话注入 / 未迁移前置拦截）得到完整覆盖。
const invoke = vi.fn()

beforeEach(() => {
  invoke.mockReset()
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: { __TAURI_INTERNALS__: { invoke } },
  })
  setActiveProjectSessionContext({
    projectId: 'project-A',
    leaseId: 'lease-A',
    projectPath: 'C:/projects/A',
  })
})

afterEach(() => {
  setActiveProjectSessionContext(null)
  Reflect.deleteProperty(globalThis, 'window')
})

describe('project-scoped IPC session transport', () => {
  it('appends the frozen active session as a named argument for project channels', async () => {
    invoke.mockResolvedValue([])

    await ipc.invoke('db:blueprint-get-all', 'C:/projects/A')

    expect(invoke).toHaveBeenCalledTimes(1)
    // 真实 Tauri core `invoke` 会带上第三个 `options` 参数，故只断言前两位
    expect(invoke.mock.calls[0]?.slice(0, 2)).toEqual(['db_blueprint_get_all', {
      expectedProjectPath: 'C:/projects/A',
      projectSession: {
        projectId: 'project-A',
        leaseId: 'lease-A',
        projectPath: 'C:/projects/A',
      },
    }])
  })

  it('allows a capability-grant filesystem request without an active project session', async () => {
    invoke.mockResolvedValue({ success: true })
    setActiveProjectSessionContext(null)

    await ipc.invoke('fs:grant-write-file', 'grant-export-1', 'chapter.txt', 'content')

    expect(invoke).toHaveBeenCalledTimes(1)
    expect(invoke.mock.calls[0]?.slice(0, 2)).toEqual(['fs_grant_write_file', {
      grantId: 'grant-export-1',
      relativePath: 'chapter.txt',
      content: 'content',
    }])
  })
})
