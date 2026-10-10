import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { page } from 'vitest/browser'

import { useLayoutStore } from '../../../stores/layout-store'
import { useLocaleStore } from '../../../stores/locale-store'
import SettingsModal from '../SettingsModal'

import {
  installTauriInternals,
  type TauriInternalsHandle,
} from '../../../../test/helpers/tauri-internals'
;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

let root: Root
let container: HTMLDivElement
let tauriInternals: TauriInternalsHandle
let configGetResult: unknown
let configSetResult: () => unknown

async function renderSection(section: 'proxy' | 'editor') {
  useLayoutStore.setState({ settingsSection: section })
  await act(async () => root.render(<SettingsModal open onClose={() => {}} />))
}

beforeEach(() => {
  useLocaleStore.setState({ locale: 'zh-CN' })
  configGetResult = { autoOpenNextChapterAfterFinalize: false }
  configSetResult = () => ({ success: true })
  tauriInternals = installTauriInternals({
    commands: {
      config_get: () => configGetResult,
      config_set: () => configSetResult(),
    },
  })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  tauriInternals.uninstall()
})

describe('settings persistence truthfulness', () => {
  it.each([
    ['business failure', () => Promise.resolve({ success: false, error: 'disk full' })],
    ['transport rejection', () => Promise.reject(new Error('IPC unavailable'))],
  ] as const)('does not report proxy settings as saved after a %s', async (_label, failure) => {
    configGetResult = { proxy: { enabled: false, type: 'http', host: '', port: 7890 } }
    configSetResult = failure
    await renderSection('proxy')

    await act(async () => page.getByRole('button', { name: '保存代理配置' }).click())

    await expect.element(page.getByText(/代理配置保存失败/)).toBeVisible()
    await expect.element(page.getByText('已保存')).not.toBeInTheDocument()
  })

  it('rolls back the auto-open-next setting when persistence fails', async () => {
    configGetResult = { autoOpenNextChapterAfterFinalize: false }
    configSetResult = () => ({ success: false, error: 'read only' })
    await renderSection('editor')
    const toggle = page.getByRole('switch', { name: '定稿后打开下一章' })
    await expect.element(toggle).not.toBeChecked()

    await act(async () => toggle.click())

    await expect.element(toggle).not.toBeChecked()
    await expect.element(page.getByText(/自动打开下一章设置保存失败/)).toBeVisible()
  })
})
