import { describe, expect, it } from 'vitest'

import { MIGRATED_CHANNELS } from '../src/shared/migrated-channels'
import { readNormalizedSource } from './source-contract'

/**
 * `src/shared/migrated-channels.ts` 是**生成物**（`pnpm run check:channels:emit`）。
 * 本测试重算同样的映射并比对，保证：
 *   1. 生成物不会过期（迁移新频道后忘了重新生成 → 测试失败）；
 *   2. 生成物不会被手改（手改会与 lib.rs / 契约不一致 → 测试失败）；
 *   3. 未迁移频道仍能被识别（集合不是「全量契约」）。
 */

/** 契约里的 invoke 频道（按所属 interface 名排除事件频道） */
function collectInvokeChannels(): Set<string> {
  const invoke = new Set<string>()
  let iface = ''
  for (const line of readNormalizedSource('../src/shared/ipc-channels.ts').split('\n')) {
    const ifaceMatch = line.match(/^export interface (\w+)/u)
    if (ifaceMatch) iface = ifaceMatch[1]
    const channel = line.match(/^\s*'([a-z0-9-]+:[a-z0-9-]+)'\s*:\s*\{/u)
    if (!channel) continue
    if (!/Event/u.test(iface)) invoke.add(channel[1])
  }
  return invoke
}

/** lib.rs 中 `generate_handler!` 已注册的命令名 */
function collectRegisteredCommands(): Set<string> {
  const commands = new Set<string>()
  for (const match of readNormalizedSource('src-tauri/src/lib.rs').matchAll(/commands::([a-z0-9_]+)/gu)) {
    commands.add(match[1])
  }
  return commands
}

const commandName = (channel: string): string => channel.replace(/[:-]/g, '_')

describe('channel migration coverage', () => {
  it('migrated-channels.ts 恰好等于 lib.rs 注册命令所覆盖的契约频道', () => {
    const registered = collectRegisteredCommands()
    const expected = [...collectInvokeChannels()]
      .filter((channel) => registered.has(commandName(channel)))
      .sort()

    expect([...MIGRATED_CHANNELS].sort()).toEqual(expected)
  })

  it('每个已迁移频道都能机械映射回一个已注册命令', () => {
    const registered = collectRegisteredCommands()
    const unresolved = [...MIGRATED_CHANNELS].filter((channel) => !registered.has(commandName(channel)))
    expect(unresolved).toEqual([])
  })

  it('已迁移集合是契约的真子集（仍存在未迁移频道）', () => {
    const invoke = collectInvokeChannels()
    expect(MIGRATED_CHANNELS.size).toBeGreaterThan(0)
    expect(MIGRATED_CHANNELS.size).toBeLessThan(invoke.size)
    for (const channel of MIGRATED_CHANNELS) {
      expect(invoke.has(channel), `${channel} 不在契约 invoke 频道中`).toBe(true)
    }
  })

  it('未迁移频道会被前置拦截（如批次 D 的 llm:list-models）', () => {
    expect(MIGRATED_CHANNELS.has('llm:list-models')).toBe(false)
    expect(MIGRATED_CHANNELS.has('kb:search')).toBe(false)
    // 已迁频道不受影响
    expect(MIGRATED_CHANNELS.has('config:get')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:project-clear-generated-data')).toBe(true)
  })
})
