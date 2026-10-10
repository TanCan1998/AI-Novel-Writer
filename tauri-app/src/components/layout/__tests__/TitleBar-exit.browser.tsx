import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { page } from 'vitest/browser'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useLocaleStore } from '../../../stores/locale-store'
import { useProjectStore } from '../../../stores/project-store'
import { useWorkflowStore } from '../../../stores/workflow-store'
import {
  registerEditorExitSaveHandler,
  useEditorStore,
} from '../../../stores/editor-store'
import TitleBar from '../TitleBar'
import {
  installTauriInternals,
  type TauriInternalsHandle,
} from '../../../../test/helpers/tauri-internals'

;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const PROJECT = 'C:\\novels\\native-exit'
const originalEditorState = useEditorStore.getState()
const originalProjectState = useProjectStore.getState()
const originalLocaleState = useLocaleStore.getState()

let root: Root
let container: HTMLDivElement
let tauriInternals: TauriInternalsHandle

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>(next => { resolve = next })
  return { promise, resolve }
}

beforeEach(() => {
  useEditorStore.getState().clearTabs()
  useEditorStore.setState({ tabs: [], activeTabId: null, draftLedgers: {} })
  useProjectStore.setState({
    currentProject: {
      id: 'native-exit',
      sessionLease: 'native-exit-lease',
      name: 'Native exit',
      path: PROJECT,
      novelConfig: {},
    } as never,
  })
  useLocaleStore.setState({ locale: 'zh-CN' })
  useWorkflowStore.setState({ activeRuns: [] })
  tauriInternals = installTauriInternals({
    commands: { window_resolve_close: { success: true } },
  })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  useEditorStore.setState(originalEditorState)
  useProjectStore.setState(originalProjectState)
  useLocaleStore.setState(originalLocaleState)
  tauriInternals.uninstall()
})

describe('TitleBar native exit settlement', () => {
  it('lets a clean system close proceed without prompting', async () => {
    await act(async () => root.render(<TitleBar />))

    await act(async () => tauriInternals.emit('window:close-requested', { requestId: 'close-clean' }))

    expect(tauriInternals.invoke).toHaveBeenCalledWith(
      'window_resolve_close',
      { requestId: 'close-clean', decision: 'proceed' },
      undefined,
    )
    await expect.element(page.getByRole('dialog')).not.toBeInTheDocument()
  })

  it('supports cancel, save, and discard without losing a failed or newly edited draft', async () => {
    useEditorStore.setState({
      tabs: [{
        id: 'draft-a',
        name: '第一章',
        type: 'chapter',
        projectKey: PROJECT,
        content: 'AB',
        contentRevision: 2,
        dirty: true,
      }],
    })
    await act(async () => root.render(<TitleBar />))

    await act(async () => tauriInternals.emit('window:close-requested', { requestId: 'close-cancel' }))
    await expect.element(page.getByRole('dialog')).toBeVisible()
    await act(async () => page.getByRole('button', { name: '取消' }).click())
    expect(tauriInternals.invoke).toHaveBeenCalledWith(
      'window_resolve_close',
      { requestId: 'close-cancel', decision: 'cancel' },
      undefined,
    )
    expect(useEditorStore.getState().tabs[0]?.dirty).toBe(true)

    registerEditorExitSaveHandler({
      tabId: 'draft-a',
      type: 'chapter',
      projectKey: PROJECT,
      save: async () => {
        useEditorStore.getState().updateTabContent('draft-a', 'ABC')
        useEditorStore.getState().settleTabSave('draft-a', { content: 'AB', contentRevision: 2 })
      },
    })
    await act(async () => tauriInternals.emit('window:close-requested', { requestId: 'close-save-race' }))
    await act(async () => page.getByRole('button', { name: '保存并退出' }).click())
    await expect.element(page.getByText('保存期间仍有未保存修改，已取消退出')).toBeVisible()
    expect(tauriInternals.invoke).not.toHaveBeenCalledWith(
      'window_resolve_close',
      { requestId: 'close-save-race', decision: 'proceed' },
      undefined,
    )
    expect(useEditorStore.getState().tabs[0]).toMatchObject({ content: 'ABC', dirty: true })
    useEditorStore.setState({
      draftLedgers: {
        config: JSON.stringify({
          version: 1,
          projects: [{ projectKey: PROJECT, baseValue: {}, draftValue: { genre: '未保存配置' } }],
        }),
      },
    })
    await act(async () => page.getByRole('button', { name: '放弃并退出' }).click())
    expect(tauriInternals.invoke).toHaveBeenCalledWith(
      'window_resolve_close',
      { requestId: 'close-save-race', decision: 'proceed' },
      undefined,
    )
    expect(tauriInternals.invoke).not.toHaveBeenCalledWith(
      'window_resolve_close',
      { requestId: 'close-save-race', decision: 'cancel' },
      undefined,
    )
    expect(tauriInternals.invoke).not.toHaveBeenCalledWith('window_close', {}, undefined)
    expect(useEditorStore.getState().tabs.some(tab => tab.dirty)).toBe(false)
    expect(JSON.parse(useEditorStore.getState().draftLedgers.config).projects).toEqual([])
    await expect.element(page.getByRole('dialog')).not.toBeInTheDocument()
  })

  it.each([
    ['business failure', () => Promise.resolve({ success: false }), '退出请求已失效，请重试'],
    ['transport rejection', () => Promise.reject(new Error('IPC unavailable')), 'IPC unavailable'],
  ] as const)('keeps unsaved changes when discard-and-exit hits a %s', async (_label, failure, errorText) => {
    useEditorStore.setState({
      tabs: [{
        id: 'draft-a',
        name: '第一章',
        type: 'chapter',
        projectKey: PROJECT,
        content: '未保存正文',
        dirty: true,
      }],
    })
    await act(async () => root.render(<TitleBar />))
    // 事件监听也走 invoke 通道，render 期的 plugin:event|listen 会先吃掉
    // once 桩，因此失败注入必须放在 render 之后。
    tauriInternals.invoke.mockImplementationOnce(failure)

    await act(async () => tauriInternals.emit('window:close-requested', { requestId: 'close-rejected' }))
    await act(async () => page.getByRole('button', { name: '放弃并退出' }).click())

    expect(useEditorStore.getState().tabs[0]).toMatchObject({
      content: '未保存正文',
      dirty: true,
    })
    await expect.element(page.getByText(errorText)).toBeVisible()
  })

  it('locks every exit action while cancellation is settling', async () => {
    useEditorStore.setState({
      tabs: [{ id: 'draft-a', name: '第一章', type: 'chapter', projectKey: PROJECT, dirty: true }],
    })
    const cancellation = deferred<{ success: boolean }>()
    await act(async () => root.render(<TitleBar />))
    // 同上：once 桩须在 render 之后注入，否则会被事件监听注册消耗。
    tauriInternals.invoke.mockReturnValueOnce(cancellation.promise)
    await act(async () => tauriInternals.emit('window:close-requested', { requestId: 'close-busy' }))

    await act(async () => page.getByRole('button', { name: '取消' }).click())

    await expect.element(page.getByRole('button', { name: '取消' })).toBeDisabled()
    await expect.element(page.getByRole('button', { name: '放弃并退出' })).toBeDisabled()
    await expect.element(page.getByRole('button', { name: '处理中...' })).toBeDisabled()
    await act(async () => cancellation.resolve({ success: true }))
    await vi.waitFor(() => expect(container.querySelector('[role="dialog"]')).toBeNull())
  })

  it('blocks native close while the current project has an active workflow', async () => {
    useEditorStore.setState({ tabs: [], draftLedgers: {} })
    useWorkflowStore.setState({
      activeRuns: [{ projectPath: PROJECT, status: 'running' }] as never,
    })
    await act(async () => root.render(<TitleBar />))

    await act(async () => tauriInternals.emit('window:close-requested', { requestId: 'close-workflow' }))

    await expect.element(page.getByText('创作任务仍在运行')).toBeVisible()
    await expect.element(page.getByText('请先等待当前创作任务完成，或在任务面板中取消任务后再退出。')).toBeVisible()
    // 渲染期仅注册事件监听；close 请求被工作流拦截，不产生任何命令调用。
    expect(tauriInternals.invoke.mock.calls.map(call => call[0])).toEqual(['plugin:event|listen'])
    expect(useWorkflowStore.getState().activeRuns[0]?.status).toBe('running')
    await expect.element(page.getByRole('button', { name: '放弃并退出' })).not.toBeInTheDocument()
    await act(async () => page.getByRole('button', { name: '知道了' }).click())
    expect(tauriInternals.invoke).toHaveBeenCalledWith(
      'window_resolve_close',
      { requestId: 'close-workflow', decision: 'cancel' },
      undefined,
    )
  })
})
