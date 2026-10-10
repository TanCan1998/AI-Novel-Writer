import { afterEach, expect, it, vi } from 'vitest'
import { page } from 'vitest/browser'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'

import { useLocaleStore } from '../../../stores/locale-store'
import { useProjectStore } from '../../../stores/project-store'
import PromptSettings from '../PromptSettings'

import {
  installTauriInternals,
  type TauriInternalsHandle,
} from '../../../../test/helpers/tauri-internals'
;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

let root: Root | undefined
let container: HTMLDivElement | undefined

let tauriInternals: TauriInternalsHandle | undefined
afterEach(async () => {
  await act(async () => root?.unmount())
  container?.remove()
  useProjectStore.setState({ currentProject: null })
  useLocaleStore.setState({ locale: 'zh-CN' })
  tauriInternals?.uninstall()
  tauriInternals = undefined
})

it('checks project prompt diagnostics for the language selected in settings', async () => {
  tauriInternals = installTauriInternals({
    commands: {
      prompt_load_global: () => {
        throw new Error('global unavailable in project diagnostic fixture')
      },
      fs_check_exists: () => true,
      fs_list_dir: () => [{
        name: 'premise.zh-CN.json',
        path: 'C:/novels/english/.lore/prompts/premise.zh-CN.json',
        isDir: false,
      }],
      fs_read_file: { success: true, content: '{invalid json' },
    },
  })
  useLocaleStore.setState({ locale: 'en-US' })
  useProjectStore.setState({
    currentProject: {
      id: 'english-project',
      name: 'English Project',
      path: 'C:/novels/english',
      sessionLease: 'lease-english',
      novelConfig: { writingLanguage: 'en-US' },
    } as never,
  })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  await act(async () => { root?.render(<PromptSettings />) })

  await vi.waitFor(() => expect(tauriInternals?.invoke).toHaveBeenCalledWith(
    'fs_read_file',
    expect.anything(),
    undefined,
  ))
  await expect.element(page.getByText(/Project prompts could not be loaded/)).not.toBeInTheDocument()

  await act(async () => page.getByRole('combobox', { name: 'Writing language to edit' }).selectOptions('zh-CN'))
  await expect.element(page.getByText(/Project prompts could not be loaded/)).toBeVisible()
})

it('keeps the project error visible when a later global retry succeeds', async () => {
  let globalAttempt = 0
  tauriInternals = installTauriInternals({
    commands: {
      prompt_load_global: () => {
        globalAttempt += 1
        if (globalAttempt === 1) throw new Error('global denied')
        return { templates: [], diagnostics: [] }
      },
      fs_check_exists: (args: { filePath: string }) => {
        if (String(args.filePath).endsWith('/.lore/prompts')) throw new Error('project denied')
        throw new Error('Unexpected IPC channel: fs:check-exists')
      },
    },
  })

  useLocaleStore.setState({ locale: 'zh-CN' })
  useProjectStore.setState({
    currentProject: {
      id: 'project-a',
      name: 'Project A',
      path: 'C:/novels/project-a',
      sessionLease: 'lease-a',
      novelConfig: {},
    } as never,
  })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  await act(async () => { root?.render(<PromptSettings />) })

  await expect.element(page.getByText(/全局提示词加载失败/)).toBeVisible()
  await expect.element(page.getByText(/项目提示词加载失败/)).toBeVisible()

  await act(async () => { root?.render(<></>) })
  await act(async () => { root?.render(<PromptSettings />) })
  await vi.waitFor(() => expect(globalAttempt).toBeGreaterThan(1))
  await expect.element(page.getByText(/项目提示词加载失败/)).toBeVisible()
})
