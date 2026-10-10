import { afterEach, describe, expect, it, vi } from 'vitest'

import type { WorkflowContext } from '../../../stores/workflow-store'
import { runPostProcessPipeline } from '../workflow-utils'

import { installTauriInternals, type TauriInternalsHandle } from '../../../../test/helpers/tauri-internals'
const projectSession = {
  projectId: 'project-1',
  leaseId: 'lease-1',
  projectPath: 'C:/novel',
} as const

function englishContext(): WorkflowContext {
  return {
    runId: 'post-process-en',
    projectPath: projectSession.projectPath,
    projectSession,
    writingLanguage: 'zh-CN',
    uiLocale: 'en-US',
    data: {},
    cancelled: false,
  }
}

let tauriInternals: TauriInternalsHandle

function stubIpcInvoke() {
  let latestRunCalls = 0
  tauriInternals = installTauriInternals({
    commands: {
      db_post_process_get_latest_run: () => (latestRunCalls++ === 0 ? null : { id: 'run-1' }),
      db_post_process_create_run: { success: true, id: 'run-1' },
      db_post_process_get_steps: [],
      db_post_process_mark_step_failed: { success: true },
    },
  })
}

afterEach(() => {
  tauriInternals?.uninstall()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('runPostProcessPipeline stopOnFailure', () => {
  it('persists the failed step and stops before any later post-processing step runs', async () => {
    stubIpcInvoke()
    const secondStep = vi.fn(async () => undefined)
    const callbacks = { log: vi.fn(), setProgress: vi.fn(), appendText: vi.fn() }

    await expect(runPostProcessPipeline(
      'C:/novel',
      'chapter_2_finalize',
      'Chapter 2 finalization',
      [
        { key: 'first', label: 'First step', critical: true, executor: async () => { throw new Error('provider timeout') } },
        { key: 'second', label: 'Second step', critical: false, executor: secondStep },
      ],
      callbacks,
      { retryCount: 1, stopOnFailure: true, cancellation: englishContext(), projectSession },
    )).rejects.toThrow('Post-processing step failed: First step — provider timeout')

    const logs = vi.mocked(callbacks.log).mock.calls.flat().join('\n')
    expect(logs).toContain('First step failed on attempt 1; retrying')
    expect(logs).not.toMatch(/⚠️|✅|❌|⏭️|💡/u)

    expect(tauriInternals.invoke).toHaveBeenCalledWith(
      'db_post_process_mark_step_failed',
      {
        runId: 'run-1',
        stepKey: 'first',
        errorMsg: 'provider timeout',
        expectedProjectPath: 'C:/novel',
        projectSession,
      },
      undefined,
    )
    expect(secondStep).not.toHaveBeenCalled()
  })

  it('keeps every step and the final summary bound to the id returned by create-run', async () => {
    const createdRunId = 'run-created-in-same-second'
    const unrelatedLatestRunId = 'run-unrelated-latest'
    let getStepsCalls = 0
    tauriInternals = installTauriInternals({
      commands: {
        db_post_process_get_latest_run: { id: unrelatedLatestRunId },
        db_post_process_create_run: { success: true, id: createdRunId },
        db_post_process_get_steps: () => {
          getStepsCalls += 1
          return getStepsCalls === 1
            ? [{
                id: 1,
                runId: createdRunId,
                stepKey: 'only',
                label: 'Only step',
                critical: true,
                ok: false,
                errorMsg: '',
                attemptCount: 0,
                completedAt: '',
                lastAttemptAt: '',
              }]
            : [{
                id: 1,
                runId: createdRunId,
                stepKey: 'only',
                label: 'Only step',
                critical: true,
                ok: true,
                errorMsg: '',
                attemptCount: 1,
                completedAt: '2026-07-25T00:00:00.000Z',
                lastAttemptAt: '2026-07-25T00:00:00.000Z',
              }]
        },
        db_post_process_mark_step_ok: { success: true },
      },
    })

    const callbacks = { log: vi.fn(), setProgress: vi.fn(), appendText: vi.fn() }
    const status = await runPostProcessPipeline(
      'C:/novel',
      'chapter_2_finalize',
      'Chapter 2 finalization',
      [{ key: 'only', label: 'Only step', critical: true, executor: async () => undefined }],
      callbacks,
      { retryCount: 0, cancellation: englishContext(), projectSession },
    )

    expect(tauriInternals.invoke.mock.calls.filter(([command]) => command === 'db_post_process_get_latest_run'))
      .toHaveLength(1)
    expect(tauriInternals.invoke).toHaveBeenCalledWith(
      'db_post_process_mark_step_ok',
      { runId: createdRunId, stepKey: 'only', expectedProjectPath: 'C:/novel', projectSession },
      undefined,
    )
    expect(tauriInternals.invoke.mock.calls.filter(([command, args]) =>
      command === 'db_post_process_get_steps' && args?.runId === createdRunId
    )).toHaveLength(2)
    expect(tauriInternals.invoke.mock.calls.some(([, args]) => args?.runId === unrelatedLatestRunId)).toBe(false)
    expect(status.steps.only.ok).toBe(true)
    const logs = vi.mocked(callbacks.log).mock.calls.flat().join('\n')
    expect(logs).toContain('Initializing post-processing run')
    expect(logs).toContain('Chapter 2 finalization post-processing summary')
    expect(logs).toContain('1/1 succeeded')
    expect(logs).not.toMatch(/初始化后处理跑批|后处理汇总|成功|⚠️|✅|❌|⏭️|💡/u)
  })

  it('fails closed before IPC when no frozen project session is supplied', async () => {
    stubIpcInvoke()

    await expect(runPostProcessPipeline(
      'C:/novel',
      'chapter_2_finalize',
      '第2章定稿',
      [],
      { log: vi.fn(), setProgress: vi.fn(), appendText: vi.fn() },
      { retryCount: 0 },
    )).rejects.toThrow('后处理缺少冻结项目会话')

    expect(tauriInternals.invoke).not.toHaveBeenCalled()
  })
})
