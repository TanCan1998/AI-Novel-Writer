#!/usr/bin/env node
/**
 * 静态核对：契约频道 ↔ 已注册 Tauri 命令。
 *
 * 迁移以 `src/shared/ipc-channels.ts` 为唯一事实源：每个 invoke 频道对应一个
 * `#[tauri::command]`，命令名 = 频道名的机械映射（`:` / `-` → `_`）。
 * 上游合并后契约已拆成模块合成（`AllInvokeChannels =
 * LegacyRosterGenerationChannels & ... & MCPChannels`），频道采集自
 * `ipc-channels.ts` 递归跟随相对 import（见 `scripts/channel-contract.mjs`）。
 * 本脚本在**不启动应用**的前提下验证该映射，用于每批迁移后的回归：
 *
 *   - `orphan`：注册了命令但契约里没有对应频道（拼写错误 / 不该存在的命令）→ 退出码 1；
 *   - `missing`：契约有频道但尚未注册（= 后续批次的工作清单）。
 *
 * 用法：
 *   node scripts/verify-channel-coverage.mjs             # 汇总 + 未迁移清单
 *   node scripts/verify-channel-coverage.mjs --quiet     # 只输出汇总
 *   node scripts/verify-channel-coverage.mjs --emit      # 额外（重新）生成
 *                                                        # src/shared/migrated-channels.ts
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { collectContractChannels } from './channel-contract.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const appRoot = path.resolve(here, '..')
const repoRoot = path.resolve(appRoot, '..')

// 契约事实源 = tauri-app 副本：G1 的 `finalization:*` 只在 Tauri 侧声明
// （上游 `src/` 未声明且保持不动，以保证上游可合并）。
// 契约闭包 = 自 `src/shared/ipc-channels.ts` 递归跟随相对 import 的全部文件。
const libFile = path.join(appRoot, 'src-tauri/src/lib.rs')
const migratedFile = path.join(appRoot, 'src/shared/migrated-channels.ts')
const quiet = process.argv.includes('--quiet')
const emit = process.argv.includes('--emit')

function readLines(file) {
  try {
    return fs.readFileSync(file, 'utf8').split('\n')
  } catch (error) {
    console.error(`无法读取 ${path.relative(repoRoot, file)}：${error.message}`)
    process.exit(2)
  }
}


/** lib.rs 中 generate_handler! 已注册的命令名 */
function collectRegistered() {
  const registered = new Set()
  for (const line of readLines(libFile)) {
    for (const match of line.matchAll(/commands::([a-z0-9_]+)/gu)) registered.add(match[1])
  }
  return registered
}

/**
 * （重新）生成 `src/shared/migrated-channels.ts`。
 * 内容未变时不写盘，避免制造无意义 diff。
 */
function emitMigratedChannels(channels) {
  const generated = [
    '/**',
    ' * 已迁移到 Tauri 侧的契约频道（**自动生成，请勿手改**）。',
    ' *',
    ' * 生成：`pnpm run check:channels --emit`',
    ' * 事实源：`src-tauri/src/lib.rs` 的 `generate_handler!` 命令注册',
    ' *         ↔ `src/shared/ipc-channels.ts` 的 invoke 频道（机械映射 `:`/`-` → `_`）。',
    ' *',
    ' * 用途：ipc-client 对未迁移频道直接给出「尚未迁移」的友好提示，',
    ' *       而不是 Tauri 原生的 `Unknown Error: Command xxx not found`。',
    ' */',
    'export const MIGRATED_CHANNELS: ReadonlySet<string> = new Set([',
    ...channels.map((channel) => `  '${channel}',`),
    '])',
    '',
  ].join('\n')

  const previous = fs.existsSync(migratedFile) ? fs.readFileSync(migratedFile, 'utf8') : ''
  if (previous === generated) return false
  fs.writeFileSync(migratedFile, generated, 'utf8')
  return true
}

const commandName = (channel) => channel.replace(/[:-]/g, '_')

// 契约里的 invoke 频道与事件频道（事件按所属 interface 名区分）。
const contract = collectContractChannels({ appRoot })
const invoke = new Set(contract.invoke.keys())
const events = contract.events
const registered = collectRegistered()

const migrated = []
const missing = []
for (const channel of [...invoke].sort()) {
  if (registered.has(commandName(channel))) migrated.push(channel)
  else missing.push(channel)
}

/** 骨架命令（无契约频道，仅用于阶段 0 联调） */
const SKELETON_COMMANDS = new Set(['app_health_check'])
const channelCommands = new Set([...invoke].map(commandName))
const orphan = [...registered]
  .filter((command) => !SKELETON_COMMANDS.has(command) && !channelCommands.has(command))
  .sort()

const byPrefix = (list) => {
  const counts = new Map()
  for (const channel of list) {
    const prefix = channel.split(':')[0]
    counts.set(prefix, (counts.get(prefix) ?? 0) + 1)
  }
  return [...counts.entries()]
    .sort((left, right) => right[1] - left[1])
    .map(([prefix, count]) => `${prefix}=${count}`)
    .join(' ')
}

console.log(`契约 invoke 频道 ${invoke.size}（事件频道 ${events.size}）`)
console.log(`已注册命令 ${registered.size} → 覆盖 invoke 频道 ${migrated.length}`)
console.log(`未迁移 invoke 频道 ${missing.length}${missing.length ? `  [${byPrefix(missing)}]` : ''}`)

if (!quiet && missing.length > 0) {
  console.log('--- 未迁移清单 ---')
  for (const channel of missing) console.log(`  ${channel}`)
}

if (orphan.length > 0) {
  console.error('--- 命令名与契约频道无法对应（必须修复）---')
  for (const command of orphan) console.error(`  ${command}`)
  process.exit(1)
}

console.log('命令名与契约频道一一对应 ✅')

if (emit) {
  const changed = emitMigratedChannels(migrated)
  console.log(
    changed
      ? `已生成 src/shared/migrated-channels.ts（${migrated.length} 个频道）`
      : `src/shared/migrated-channels.ts 已是最新（${migrated.length} 个频道）`,
  )
}
