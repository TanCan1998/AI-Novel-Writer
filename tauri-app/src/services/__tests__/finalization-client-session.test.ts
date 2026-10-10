import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { setActiveProjectSessionContext } from '../../shared/project-session-context'

// 迁移后底层为 `ipc.invoke`（Tauri）。这里 mock 掉 ipc-client，断言调用参数与会话冻结语义。
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('../ipc-client', () => ({ ipc: { invoke } }))

import { retryFinalizationPublication } from '../finalization-client'

const frozenSession = {
  projectId: 'project-a',
  leaseId: 'lease-a',
  projectPath: 'C:\\NovelA',
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue({ success: true, committed: true })
  // 被测模块会比较活动会话与调用方冻结会话，须先建立活动会话。
  setActiveProjectSessionContext(frozenSession)
})

afterEach(() => {
  setActiveProjectSessionContext(null)
})

describe('retryFinalizationPublication', () => {
  it('当前会话缺失时拒绝写入', async () => {
    // Tauri 版无 window 桥接守卫；唯一守卫是活动会话与传入会话的一致性比较。
    setActiveProjectSessionContext(null)
    await expect(retryFinalizationPublication('finalization-1', frozenSession)).rejects.toThrow('项目会话已变化')
    expect(invoke).not.toHaveBeenCalled()
  })
  it('sends the caller-frozen session instead of recapturing one later', async () => {
    await retryFinalizationPublication('finalization-1', frozenSession)

    expect(invoke).toHaveBeenCalledWith(
      'finalization:retry',
      'finalization-1',
      frozenSession,
    )
  })

  it('rejects a retry after the same path is reopened under a new lease', async () => {
    setActiveProjectSessionContext({
      ...frozenSession,
      leaseId: 'lease-b',
      projectPath: 'c:/NovelA/.',
    })

    await expect(retryFinalizationPublication('finalization-1', frozenSession))
      .rejects.toThrow('项目会话已变化')
    expect(invoke).not.toHaveBeenCalled()
  })
})
