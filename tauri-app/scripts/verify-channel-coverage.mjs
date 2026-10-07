#!/usr/bin/env node
/**
 * 静态核对：契约频道 ↔ 已注册 Tauri 命令。
 *
 * 迁移以 `src/shared/ipc-channels.ts` 为唯一事实源：每个 invoke 频道对应一个
 * `#[tauri::command]`，命令名 = 频道名的机械映射（`:` / `-` → `_`）。
 * 本脚本在**不启动应用**的前提下验证该映射，用于每批迁移后的回归：
 *
 *   - `orphan`：注册了命令但契约里没有对应频道（拼写错误 / 不该存在的命令）→ 退出码 1；
 *   - `missing`：契约有频道但尚未注册（= 后续批次的工作清单）。
 *
 * 用法：
 *   node scripts/verify-channel-coverage.mjs             # 汇总 + 未迁移清单
 *   node scripts/verify-channel-coverage.mjs --quiet     # 只输出汇总
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const appRoot = path.resolve(here, '..')
const repoRoot = path.resolve(appRoot, '..')

const channelsFile = path.join(repoRoot, 'src/shared/ipc-channels.ts')
const libFile = path.join(appRoot, 'src-tauri/src/lib.rs')

const quiet = process.argv.includes('--quiet')

function readLines(file) {
  try {
    return fs.readFileSync(file, 'utf8').split('\n')
  } catch (error) {
    console.error(`无法读取 ${path.relative(repoRoot, file)}：${error.message}`)
    process.exit(2)
  }
}

/** 契约里的 invoke 频道与事件频道（事件按所属 interface 名区分） */
function collectChannels() {
  const invoke = new Set()
  const events = new Set()
  let iface = ''
  for (const line of readLines(channelsFile)) {
    const ifaceMatch = line.match(/^export interface (\w+)/u)
    if (ifaceMatch) iface = ifaceMatch[1]
    const channel = line.match(/^\s*'([a-z0-9-]+:[a-z0-9-]+)'\s*:\s*\{/u)
    if (!channel) continue
    if (/Event/u.test(iface)) events.add(channel[1])
    else invoke.add(channel[1])
  }
  return { invoke, events }
}

/** lib.rs 中 generate_handler! 已注册的命令名 */
function collectRegistered() {
  const registered = new Set()
  for (const line of readLines(libFile)) {
    for (const match of line.matchAll(/commands::([a-z0-9_]+)/gu)) registered.add(match[1])
  }
  return registered
}

const commandName = (channel) => channel.replace(/[:-]/g, '_')

const { invoke, events } = collectChannels()
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
