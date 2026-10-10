#!/usr/bin/env node
/**
 * 契约频道采集（`src/shared/ipc-channels.ts` 事实源的唯一解析入口）。
 *
 * 上游合并后契约从单文件改成模块合成（`AllInvokeChannels =
 * LegacyRosterGenerationChannels & CharacterAvatarChannels & ... & MCPChannels`），
 * 频道分散在多个契约模块里。本模块自 `src/shared/ipc-channels.ts` 出发，
 * 递归跟随所有相对 import（`./`、`../`），收集全部契约频道。
 *
 * 频道归属规则保持不变：频道写在 `export interface Xxx` 里，
 * iface 名含 `Event` → 事件频道，否则 invoke 频道。
 *
 * 供 `scripts/verify-channel-coverage.mjs` 与
 * `test/channel-migration-coverage.test.ts` 共用，保证两处口径一致。
 */
import fs from 'node:fs'
import path from 'node:path'

/** 相对 import（`from './x'` / `import './x'`）。 */
const IMPORT_PATTERN = /(?:\bfrom\s*|\bimport\s*)['"](\.[^'"]+)['"]/g
/** 频道容器声明。 */
const INTERFACE_PATTERN = /^export interface (\w+)/gmu
/** 频道键：`'domain:action': { ... }` 或 `'domain:action': SomeType`。 */
const CHANNEL_PATTERN = /^\s*'([a-z0-9-]+:[a-z0-9-]+)'\s*:\s*[{A-Za-z]/gmu
/** 事件频道按所属 interface 名区分。 */
const EVENT_PATTERN = /Event/u

/**
 * 解析相对 import：`<dir>/<spec>`、`<spec>.ts`、`<spec>.tsx`、`<spec>/index.ts`。
 */
function resolveImport(spec, dir) {
  const base = path.resolve(dir, spec)
  for (const candidate of [
    base,
    `${base}.ts`,
    `${base}.tsx`,
    path.join(base, 'index.ts'),
  ]) {
    if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) {
      return candidate
    }
  }
  return null
}

/**
 * 收集契约频道。
 *
 * @param {{ appRoot: string }} options `tauri-app/` 根目录。
 * @returns {{
 *   invoke: Map<string, string>,
 *   events: Set<string>,
 *   files: string[],
 * }}
 *   `invoke`：invoke 频道 → 契约源码块（自频道键行起至下一频道键行，
 *   供 `args` 定位参数个数推断）；`events`：事件频道集合；
 *   `files`：闭包内全部文件（绝对路径，含起点）。
 */
export function collectContractChannels({ appRoot }) {
  const start = path.join(appRoot, 'src/shared/ipc-channels.ts')
  const srcRoot = path.join(appRoot, 'src')

  const invoke = new Map()
  const events = new Set()
  const files = []
  const seen = new Set()
  const stack = [start]

  while (stack.length > 0) {
    const file = stack.pop()
    if (seen.has(file)) continue
    seen.add(file)
    files.push(file)

    const source = fs.readFileSync(file, 'utf8')

    // 跟随相对 import（`seen` 去重，环安全）。
    for (const match of source.matchAll(IMPORT_PATTERN)) {
      const resolved = resolveImport(match[1], path.dirname(file))
      if (resolved && resolved.startsWith(srcRoot)) stack.push(resolved)
    }

    // 频道归属：频道键属于其之前最近的 `export interface` 声明。
    const interfaces = [...source.matchAll(INTERFACE_PATTERN)].map((match) => ({
      index: match.index ?? 0,
      name: match[1],
    }))
    const headers = [...source.matchAll(CHANNEL_PATTERN)]
    headers.forEach((header, index) => {
      const startOffset = header.index ?? 0
      const endOffset =
        index + 1 < headers.length
          ? headers[index + 1].index ?? source.length
          : source.length
      const block = source.slice(startOffset, endOffset)
      let iface = ''
      for (const declaration of interfaces) {
        if (declaration.index < startOffset) iface = declaration.name
        else break
      }
      if (EVENT_PATTERN.test(iface)) events.add(header[1])
      else invoke.set(header[1], block)
    })
  }

  return { invoke, events, files }
}
