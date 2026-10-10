import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  toastError: vi.fn(),
}))

vi.mock('../../../ui/Toast', () => ({
  toast: { error: mocks.toastError },
}))

import {
  confirmCurrentProjectSession,
  openChapterFile,
} from '../sidebar-file-openers'
import { setActiveProjectSessionContext } from '../../../../shared/project-session-context'
import { useEditorStore } from '../../../../stores/editor-store'
import { useProjectStore } from '../../../../stores/project-store'

import { installTauriInternals, type TauriInternalsHandle } from '../../../../../test/helpers/tauri-internals'
const projectPath = 'C:\\novels\\same-project'

const projectWithLease = (leaseId: string) => ({
  id: 'same-project-id',
  sessionLease: leaseId,
  name: 'Same project',
  path: projectPath,
  novelConfig: {},
}) as never

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

let tauriInternals: TauriInternalsHandle
beforeEach(() => {
  mocks.toastError.mockReset()
  useEditorStore.getState().clearTabs()
  useProjectStore.setState({ currentProject: projectWithLease('lease-A') })
  setActiveProjectSessionContext({
    projectId: 'same-project-id',
    leaseId: 'lease-A',
    projectPath,
  })
})

afterEach(() => {
  tauriInternals?.uninstall()
  vi.unstubAllGlobals()
  useEditorStore.getState().clearTabs()
  useProjectStore.setState({ currentProject: null })
  setActiveProjectSessionContext(null)
})

describe('sidebar file openers keep the original project session', () => {
  it('does not create a saved blank chapter tab when fs reports success:false', async () => {
    tauriInternals = installTauriInternals({
      commands: {
        fs_read_file: async () => ({ success: false, content: '', error: 'read denied' }),
      },
    })
    await openChapterFile(`${projectPath}\\manuscript\\chapter_1.md`, '第1章')

    expect(useEditorStore.getState().tabs).toEqual([])
    expect(tauriInternals.invoke).toHaveBeenCalledOnce()
  })

  it('drops a delayed same-path read after reopening with a new lease', async () => {
    const delayedRead = deferred<{ success: boolean; content: string }>()
    tauriInternals = installTauriInternals({
      commands: {
        fs_read_file: () => delayedRead.promise,
      },
    })
    const opening = openChapterFile(`${projectPath}\\manuscript\\chapter_1.md`, '第1章')
    await vi.waitFor(() => expect(tauriInternals.invoke).toHaveBeenCalledOnce())
    useProjectStore.setState({ currentProject: projectWithLease('lease-B') })
    setActiveProjectSessionContext({
      projectId: 'same-project-id',
      leaseId: 'lease-B',
      projectPath,
    })
    delayedRead.resolve({ success: true, content: 'stale content' })

    await opening

    expect(useEditorStore.getState().tabs).toEqual([])
  })

  it('rejects a confirmation that resolves after the same path is reopened with a new lease', async () => {
    const delayedConfirmation = deferred<boolean>()
    const confirmation = confirmCurrentProjectSession(
      projectWithLease('lease-A'),
      () => delayedConfirmation.promise,
    )

    useProjectStore.setState({ currentProject: projectWithLease('lease-B') })
    setActiveProjectSessionContext({
      projectId: 'same-project-id',
      leaseId: 'lease-B',
      projectPath,
    })
    delayedConfirmation.resolve(true)

    await expect(confirmation).resolves.toBeNull()
  })
})
