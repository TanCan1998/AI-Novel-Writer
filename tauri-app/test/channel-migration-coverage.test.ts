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

/** 契约里的 invoke 频道（按所属 interface 名排除事件频道）。
 *
 * 迁移期契约事实源 = **tauri-app 副本**（工作目录 = tauri-app/）：
 * G1 的 `finalization:*` 只在 Tauri 侧声明，上游 `src/` 保持不动以保上游可合并。
 */
function collectInvokeChannels(): Set<string> {
  const invoke = new Set<string>()
  let iface = ''
  for (const line of readNormalizedSource('src/shared/ipc-channels.ts').split('\n')) {
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

/**
 * 从契约源码块中数出一个频道的 `args` 定位参数个数（支持多行数组与嵌套对象类型）。
 *
 * 先剔除泛型实参（`Record<string, unknown>` 这类 `<...>` 内的逗号不是参数分隔符），
 * 再按括号深度统计顶层逗号。
 */
function argsArity(block: string): number {
  const sanitized = block.replace(/<[^<>]*>/gu, '')
  const start = sanitized.indexOf('args:')
  if (start === -1) return 0
  const open = sanitized.indexOf('[', start)
  if (open === -1) return 0
  let depth = 0
  let commas = 0
  let lastTopLevelComma = -1
  let close = -1
  for (let index = open; index < sanitized.length; index += 1) {
    const character = sanitized[index]
    if (character === '[' || character === '{' || character === '(') depth += 1
    else if (character === ']' || character === '}' || character === ')') {
      depth -= 1
      if (depth === 0) { close = index; break }
    } else if (character === ',' && depth === 1) {
      commas += 1
      lastTopLevelComma = index
    }
  }
  if (close === -1) return 0
  const inner = sanitized.slice(open + 1, close)
  if (inner.trim() === '') return 0
  // 多行元组类型常带尾逗号（`a: string,\n]`），尾逗号不构成额外参数。
  const tail = lastTopLevelComma === -1 ? '' : inner.slice(lastTopLevelComma - open - 1)
  const hasTrailingComma = lastTopLevelComma !== -1 && tail.replace(/^,/u, '').trim() === ''
  return hasTrailingComma ? commas : commas + 1
}

/** 已迁移频道的契约参数个数（只统计 invoke 频道：事件频道不在 MIGRATED_CHANNELS 内）。 */
function collectMigratedArgCounts(): Map<string, number> {
  const source = readNormalizedSource('src/shared/ipc-channels.ts')
  const headers = [...source.matchAll(/^\s*'([a-z0-9-]+:[a-z0-9-]+)'\s*:\s*\{/gmu)]
  const counts = new Map<string, number>()
  headers.forEach((header, index) => {
    const channel = header[1]
    if (!MIGRATED_CHANNELS.has(channel)) return
    const blockEnd = index + 1 < headers.length ? headers[index + 1].index ?? source.length : source.length
    counts.set(channel, argsArity(source.slice(header.index ?? 0, blockEnd)))
  })
  return counts
}

/** `ipc-client.ts` 中 `CHANNEL_ARG_NAMES` 已登记的频道 → 参数名序列。 */
function collectRegisteredArgNames(): Map<string, string[]> {
  // 注意：此处是 Tauri 侧客户端（工作目录 = tauri-app/），与上式契约同源。
  const source = readNormalizedSource('src/services/ipc-client.ts')
  const registered = new Map<string, string[]>()
  const pattern = /^\s*'([a-z0-9-]+:[a-z0-9-]+)'\s*:\s*\[([^\]]*)\]/gmu
  for (const match of source.matchAll(pattern)) {
    const names = match[2]
      .split(',')
      .map((entry) => entry.trim().replace(/^['"]|['"]$/gu, ''))
      .filter(Boolean)
    registered.set(match[1], names)
  }
  return registered
}

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

  it('迁移状态断言：已迁频道在集合内，批次 G2 依赖的频道仍被前置拦截', () => {
    // 批次 G1：作者原稿导入已迁移
    expect(MIGRATED_CHANNELS.has('dialog:select-novel-files')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:import-run-author-preview')).toBe(true)
    // 批次 G2a：导入运行读面已迁移
    expect(MIGRATED_CHANNELS.has('db:import-run-get')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:import-run-list-resumable')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:import-run-list-chapters')).toBe(true)
    // 仍未迁移（G2b/G3：导入运行写面与批次推进）
    expect(MIGRATED_CHANNELS.has('db:import-run-prepare-inspection')).toBe(false)
    expect(MIGRATED_CHANNELS.has('db:import-run-finalize-parsing')).toBe(false)
    // 已迁频道不受影响
    expect(MIGRATED_CHANNELS.has('config:get')).toBe(true)
    expect(MIGRATED_CHANNELS.has('llm:list-models')).toBe(true)
    expect(MIGRATED_CHANNELS.has('llm:begin-execution-lease')).toBe(true)
    expect(MIGRATED_CHANNELS.has('llm:close-execution-lease')).toBe(true)
    // 批次 D2-b：生成 / 流式 / 取消
    expect(MIGRATED_CHANNELS.has('llm:generate')).toBe(true)
    expect(MIGRATED_CHANNELS.has('llm:generate-stream')).toBe(true)
    expect(MIGRATED_CHANNELS.has('llm:cancel')).toBe(true)
    // 批次 D2-c：llm:* 收口（连通性探测 + 模型发现，llm: 前缀至此全部迁移）
    expect(MIGRATED_CHANNELS.has('llm:discover-models')).toBe(true)
    expect(MIGRATED_CHANNELS.has('llm:test-connection')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:project-clear-generated-data')).toBe(true)
    // 批次 F1：一致性豁免（consistency-exemption 子域）
    expect(MIGRATED_CHANNELS.has('db:consistency-exemption-list')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:consistency-exemption-save')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:consistency-exemption-revoke')).toBe(true)
    // 批次 F1：叙事线索（narrative-thread 子域）
    expect(MIGRATED_CHANNELS.has('db:narrative-thread-list')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:narrative-thread-list-relevant')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:narrative-thread-plan-create')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:narrative-thread-plan-update')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:narrative-thread-plan-delete')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:narrative-thread-event-confirm')).toBe(true)
    // 批次 F1：剧情树（plot-tree 子域）
    expect(MIGRATED_CHANNELS.has('db:plot-tree-read')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:plot-tree-save')).toBe(true)
    expect(MIGRATED_CHANNELS.has('db:plot-tree-clear')).toBe(true)
    // 批次 F2-3：知识库（kb:* 15 频道 + knowledge 文件选择）已迁移
    expect(MIGRATED_CHANNELS.has('kb:search')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:search-writing-context')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:search-with-scope')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:import-document')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:import-folder')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:import-text')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:import-planning-text')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:import-reference-text')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:list-documents')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:remove-document')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:clear-all')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:stats')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:get-vectorless-count')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:get-vector-rebuild-status')).toBe(true)
    expect(MIGRATED_CHANNELS.has('kb:backfill-vectors')).toBe(true)
    expect(MIGRATED_CHANNELS.has('dialog:select-knowledge-files')).toBe(true)
    expect(MIGRATED_CHANNELS.has('dialog:select-knowledge-folder')).toBe(true)
    // 批次 E（G1）：定稿提交 / 实体稿重试（finalization:* 2 频道）
    expect(MIGRATED_CHANNELS.has('finalization:commit')).toBe(true)
    expect(MIGRATED_CHANNELS.has('finalization:retry')).toBe(true)
    // 批次 H：应用数据域（prompt:* 3 + skills:* 4）
    expect(MIGRATED_CHANNELS.has('prompt:load-global')).toBe(true)
    expect(MIGRATED_CHANNELS.has('prompt:save-global')).toBe(true)
    expect(MIGRATED_CHANNELS.has('prompt:delete-global')).toBe(true)
    expect(MIGRATED_CHANNELS.has('skills:list-user')).toBe(true)
    expect(MIGRATED_CHANNELS.has('skills:inspect-github')).toBe(true)
    expect(MIGRATED_CHANNELS.has('skills:install-github')).toBe(true)
    expect(MIGRATED_CHANNELS.has('skills:uninstall-user')).toBe(true)
    // 批次 H（H3）：应用更新（update:* 6 频道）
    expect(MIGRATED_CHANNELS.has('update:get-state')).toBe(true)
    expect(MIGRATED_CHANNELS.has('update:check')).toBe(true)
    expect(MIGRATED_CHANNELS.has('update:download')).toBe(true)
    expect(MIGRATED_CHANNELS.has('update:open-release')).toBe(true)
    expect(MIGRATED_CHANNELS.has('update:defer-reminder')).toBe(true)
    expect(MIGRATED_CHANNELS.has('update:quit-and-install')).toBe(true)
  })

  /**
   * 回归防线：`ipc-client.ts` 的 `buildNamedArgs` 对「带参但未登记参数名」的频道会直接抛错，
   * 这类漏登记只有落到真机点开对应功能时才会暴露。此测试把契约的参数个数与登记表机械比对，
   * 使漏登记 / 多登记 / 参数个数不符在 `pnpm test` 阶段即失败。
   */
  it('已迁移且带参的频道都登记了正确数量的 invoke 参数名', () => {
    const registered = collectRegisteredArgNames()
    const problems: string[] = []
    for (const [channel, arity] of collectMigratedArgCounts()) {
      const names = registered.get(channel)
      if (arity === 0) {
        if (names) problems.push(`${channel}：无参频道不应登记（登记了 ${names.length} 个）`)
        continue
      }
      if (!names) {
        problems.push(`${channel}：带参 ${arity} 个但未在 CHANNEL_ARG_NAMES 登记`)
        continue
      }
      if (names.length !== arity) {
        problems.push(`${channel}：登记 ${names.length} 个，契约要求 ${arity} 个`)
      }
    }
    expect(problems).toEqual([])
  })

  it('参数名登记表不得包含未迁移频道或非法参数名', () => {
    const problems: string[] = []
    for (const [channel, names] of collectRegisteredArgNames()) {
      if (!MIGRATED_CHANNELS.has(channel)) problems.push(`${channel}：频道尚未迁移却已登记参数名`)
      for (const name of names) {
        if (!/^[a-z][a-zA-Z0-9]*$/u.test(name)) problems.push(`${channel}：参数名 ${name} 不是 lowerCamelCase`)
      }
    }
    expect(problems).toEqual([])
  })
})
