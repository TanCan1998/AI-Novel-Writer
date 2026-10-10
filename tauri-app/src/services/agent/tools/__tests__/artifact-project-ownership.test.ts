import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useProjectStore } from '../../../../stores/project-store'
import { createAgentExecutionContext } from '../project-context'
import { writeFileTool } from '../write-file.tool'

import { installTauriInternals, type TauriInternalsHandle } from '../../../../../test/helpers/tauri-internals'
const projectAPath = 'C:\\novels\\A'

let tauriInternals: TauriInternalsHandle
beforeEach(() => {
  useProjectStore.setState({
    currentProject: {
      id: 'A',
      sessionLease: 'lease-A',
      name: 'A',
      path: projectAPath,
      novelConfig: {},
    } as never,
  })
})

afterEach(() => {
  tauriInternals?.uninstall()
  vi.unstubAllGlobals()
  useProjectStore.setState({ currentProject: null })
})

describe('tool artifact project ownership', () => {
  it('freezes the tool-time project identity into every created artifact', async () => {
    tauriInternals = installTauriInternals({
      commands: { fs_write_file: { success: true } },
    })
    const result = await writeFileTool.execute({
      file_path: 'chapters/1.md',
      content: 'chapter one',
    }, createAgentExecutionContext())

    expect(tauriInternals.invoke).toHaveBeenCalledWith(
      'fs_write_file',
      {
        filePath: `${projectAPath}/chapters/1.md`,
        content: 'chapter one',
        expectedProjectPath: projectAPath,
        projectSession: expect.objectContaining({ projectId: 'A', leaseId: 'lease-A' }),
      },
      undefined,
    )
    expect(result.artifacts).toEqual([
      expect.objectContaining({
        type: 'file_modified',
        projectPath: projectAPath,
        projectSession: expect.objectContaining({
          projectId: 'A',
          leaseId: 'lease-A',
          projectPath: projectAPath,
        }),
      }),
    ])
    expect(Object.isFrozen(result.artifacts?.[0])).toBe(true)
    expect(Object.isFrozen(result.artifacts?.[0].projectSession)).toBe(true)
  })

  it('refuses an aborted write before invoking the authoritative filesystem boundary', async () => {
    tauriInternals = installTauriInternals({
      commands: { fs_write_file: { success: true } },
    })
    const controller = new AbortController()
    controller.abort()

    await expect(writeFileTool.execute({
      file_path: 'chapters/1.md',
      content: 'must not be written',
    }, {
      ...createAgentExecutionContext(),
      abortSignal: controller.signal,
    })).rejects.toThrow(/取消|cancel/u)

    expect(tauriInternals.invoke).not.toHaveBeenCalled()
  })

  it('reports the original write receipt after the response is delayed across a project switch', async () => {
    let finishWrite: (() => void) | undefined
    let commits = 0
    tauriInternals = installTauriInternals({
      commands: {
        fs_write_file: async () => {
          commits++
          await new Promise<void>(resolve => { finishWrite = resolve })
          return { success: true }
        },
      },
    })
    const executionContext = createAgentExecutionContext()
    const resultPromise = writeFileTool.execute({
      file_path: 'chapters/1.md',
      content: 'committed once',
    }, executionContext)
    await vi.waitFor(() => expect(commits).toBe(1))

    useProjectStore.setState({
      currentProject: {
        id: 'B', sessionLease: 'lease-B', name: 'B', path: 'C:\\novels\\B', novelConfig: {},
      } as never,
    })
    finishWrite?.()

    await expect(resultPromise).resolves.toMatchObject({
      success: true,
      artifacts: [expect.objectContaining({
        projectPath: projectAPath,
        projectSession: expect.objectContaining({ projectId: 'A', leaseId: 'lease-A' }),
      })],
    })
    expect(commits).toBe(1)
  })

  it.each([
    ['not_committed', false],
    ['unknown', false],
    ['committed', true],
  ] as const)('preserves a %s write result returned by the filesystem controller', async (commitState, expectedSuccess) => {
    tauriInternals = installTauriInternals({
      commands: {
        fs_write_file: () => ({ success: false, commitState, error: 'write receipt detail' }),
      },
    })

    const result = await writeFileTool.execute({
      file_path: 'chapters/1.md',
      content: 'chapter one',
    }, createAgentExecutionContext())

    expect(result).toMatchObject({ success: expectedSuccess, commitState })
    if (commitState === 'committed') {
      expect(result.artifacts).toEqual([expect.objectContaining({
        type: 'file_modified',
        projectPath: projectAPath,
      })])
    } else {
      expect(result.artifacts).toBeUndefined()
    }
  })
})
