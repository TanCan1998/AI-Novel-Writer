import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * IPC 入参结构体契约回归（源码扫描）。
 *
 * 背景（2026-10-09 GUI 冒烟发现的真实缺陷）：`chapter:delete-finalized` 的入参结构体
 * `DeleteFinalizedChapterRequest` 缺少 `#[serde(rename_all = "camelCase")]`。渲染层发
 * `{ draftId, chapterNumber }`，Tauri 在**参数反序列化**阶段就失败：
 *
 *   Unknown Error: invalid args `request` for command `chapter_delete_finalized`:
 *   missing field `draft_id`
 *
 * 该失败是 invoke **reject**，而调用点未 catch → 表现成「确认框关闭但什么都没发生」
 * 的静默失败（后端连一行记录都不会落）。单元测试用 Rust 直接构造结构体，无法覆盖。
 *
 * 规则：任何派生 `Deserialize` 且含**下划线字段名**的结构体，必须显式声明
 * `#[serde(rename_all = "camelCase")]`；确实只在进程内部使用的结构体须列入白名单。
 *
 * 注：字段名全是单个单词（无下划线）的结构体不受影响 —— `rename_all` 对它们是恒等的。
 */

/** 内部结构体白名单：不会作为 IPC 参数/载荷跨进程边界。 */
const INTERNAL_ONLY_STRUCTS = new Set(['IdMapSnapshot'])

const SRC_ROOT = 'src-tauri/src'

interface Candidate {
  file: string
  line: number
  name: string
  snakeFields: string[]
  hasCamelCase: boolean
}

function rustFiles(dir: string): string[] {
  const files: string[] = []
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry)
    if (statSync(full).isDirectory()) files.push(...rustFiles(full))
    else if (entry.endsWith('.rs')) files.push(full)
  }
  return files
}

function scanDeserializeStructs(): Candidate[] {
  const found: Candidate[] = []
  for (const file of rustFiles(SRC_ROOT)) {
    const lines = readFileSync(file, 'utf8').split('\n')
    lines.forEach((line, index) => {
      const derive = line.match(/^#\[derive\((.+)\)\]\s*$/u)
      if (!derive || !/Deserialize/u.test(derive[1])) return

      const attrs: string[] = []
      let cursor = index + 1
      while (cursor < lines.length && /^\s*#\[/u.test(lines[cursor])) {
        attrs.push(lines[cursor])
        cursor += 1
      }

      const structLine = (lines[cursor] ?? '').trim()
      const nameMatch = structLine.match(/^(?:pub(?:\([^)]*\))?\s+)?struct\s+(\w+)/u)
      if (!nameMatch) return

      const snakeFields: string[] = []
      for (let k = cursor + 1; k < lines.length; k += 1) {
        if (/^\}/u.test(lines[k])) break
        const field = lines[k].match(/^\s*(?:pub(?:\([^)]*\))?\s+)?([a-z][a-zA-Z0-9_]*)\s*:/u)
        if (field && field[1].includes('_')) snakeFields.push(field[1])
      }

      found.push({
        file: relative('.', file).replace(/\\/gu, '/'),
        line: index + 1,
        name: nameMatch[1],
        snakeFields,
        hasCamelCase: attrs.some(attribute => /rename_all\s*=\s*"camelCase"/u.test(attribute)),
      })
    })
  }
  return found
}

describe('IPC 入参结构体契约', () => {
  it('含下划线字段的 Deserialize 结构体必须显式 camelCase（或列入内部白名单）', () => {
    const candidates = scanDeserializeStructs()
    // 防「扫描失效导致空跑」：当前仓库有数十个 Deserialize 结构体
    expect(candidates.length).toBeGreaterThan(50)

    const problems = candidates
      .filter(candidate => (
        candidate.snakeFields.length > 0
        && !candidate.hasCamelCase
        && !INTERNAL_ONLY_STRUCTS.has(candidate.name)
      ))
      .map(candidate => (
        `${candidate.file}:${candidate.line} ${candidate.name} `
        + `缺 #[serde(rename_all = "camelCase")]（字段：${candidate.snakeFields.join(', ')}）`
      ))

    expect(problems).toEqual([])
  })
})
