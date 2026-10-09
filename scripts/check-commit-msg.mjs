#!/usr/bin/env node
/* global process, console */

/**
 * 提交消息卫生检查 —— 拦截 BOM、CRLF、行尾空白与不合规的语义化前缀。
 *
 * ## 为什么需要这个脚本
 *
 * 2026-10-08（批次 D2-c）有两个提交的消息被 PowerShell 5.1 的
 * `Set-Content -Encoding UTF8` 写入（该版本默认写 UTF-8 BOM），首 3 字节变成
 * `EF BB BF`。后果：
 *
 *   - `git log` 渲染成 `﻿feat(tauri): …`，肉眼几乎看不出；
 *   - 类型前缀的真实内容是 `"\uFEFFfeat"` 而**不是** `"feat"`，
 *     任何按 `type(scope):` 解析的工具都识别不到；
 *   - 只有 `git cat-file commit <sha>` 的字节级比对才能定位。
 *
 * 两个提交当时尚未推送，已用 `git commit-tree` 重放剥离 BOM
 * （`tree` / 作者 / 提交者 / 时间戳逐字节保留，仅 SHA 变化）。
 * 本脚本用于防止复发：**本地 hook 与 CI 共用同一份检查逻辑**。
 *
 * ## 检查项
 *
 * 编码卫生（对所有提交，含 `Merge`）：BOM、CR 字符、行尾空白、空消息。
 * 结构（对非豁免提交）：`type(scope): 描述` 前缀、首行显示宽度、首行与正文之间的空行。
 * 豁免：`Merge `、`Revert `、`fixup! `、`squash! ` 开头的首行只做编码卫生检查
 * （上游历史与合并提交不受本仓库提交规范约束）。
 *
 * ## 用法
 *
 *   node scripts/check-commit-msg.mjs <msgfile>        # git commit-msg hook
 *   node scripts/check-commit-msg.mjs --file <msgfile>
 *   node scripts/check-commit-msg.mjs --stdin
 *   node scripts/check-commit-msg.mjs --range <base>..HEAD [--ignore-author <email>]...
 *
 *   --range          校验一段提交范围（CI 用），逐提交读取 `git cat-file commit` 原始字节
 *   --ignore-author  跳过该提交者邮箱（可重复）。用于「PR 分支合并了 upstream，
 *                    带入上游不合规历史」的场景，避免误报
 *   --json           以 JSON 输出结果
 *   --help
 *
 * 退出码：0 全部通过；1 有不合规提交；2 用法错误。
 *
 * 挂本地 hook（可选，不入库）：
 *
 *   printf '#!/bin/sh\nexec node "$(git rev-parse --show-toplevel)/scripts/check-commit-msg.mjs" "$1"\n' \
 *     > .git/hooks/commit-msg && chmod +x .git/hooks/commit-msg
 */

import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export const COMMIT_TYPES = Object.freeze([
  'feat',
  'fix',
  'chore',
  'docs',
  'refactor',
  'test',
  'perf',
  'build',
  'ci',
  'style',
  'revert',
])

/** 语义化首行：`type(scope): 描述`，scope 为小写字母/数字/连字符。 */
export const SUBJECT_PATTERN = new RegExp(
  `^(?:${COMMIT_TYPES.join('|')})(?:\\([a-z0-9][a-z0-9-]*\\))?: \\S`,
)

/** 合并/回滚/自动压缩类提交，只做编码卫生检查。 */
export const EXEMPT_SUBJECT_PATTERN = /^(?:Merge |Revert |fixup! |squash! )/

/** 首行显示宽度上限（CJK 按 2 列计）。 */
export const MAX_SUBJECT_WIDTH = 120

const BOM_BYTES = Object.freeze([0xef, 0xbb, 0xbf])
const BOM_CHAR = '\uFEFF'
const MAX_BUFFER = 64 * 1024 * 1024

/** 全角/宽字符区间，用于把中文首行换算成终端显示宽度。 */
const FULL_WIDTH_RANGES = Object.freeze([
  [0x1100, 0x115f],
  [0x2e80, 0x303e],
  [0x3041, 0x33ff],
  [0x3400, 0x4dbf],
  [0x4e00, 0x9fff],
  [0xa000, 0xa4cf],
  [0xac00, 0xd7a3],
  [0xf900, 0xfaff],
  [0xfe30, 0xfe6f],
  [0xff00, 0xff60],
  [0xffe0, 0xffe6],
  [0x1f300, 0x1f64f],
  [0x1f900, 0x1f9ff],
  [0x20000, 0x3fffd],
])

const USAGE = `提交消息卫生检查（BOM / CRLF / 行尾空白 / 语义化前缀）

用法：
  node scripts/check-commit-msg.mjs <msgfile>              git commit-msg hook
  node scripts/check-commit-msg.mjs --file <msgfile>
  node scripts/check-commit-msg.mjs --stdin
  node scripts/check-commit-msg.mjs --range <base>..HEAD [--ignore-author <email>]...

选项：
  --range <rev-range>      校验提交范围内每个提交（CI 模式）
  --ignore-author <email>  跳过该提交者邮箱，可重复
  --json                   以 JSON 输出
  --help                   显示本帮助

退出码：0 通过；1 不合规；2 用法错误。`

/** 终端显示宽度：CJK 等宽字符按 2 列计。 */
export function displayWidth(text) {
  let width = 0
  for (const char of String(text)) {
    const code = char.codePointAt(0)
    const wide = FULL_WIDTH_RANGES.some(([low, high]) => code >= low && code <= high)
    width += wide ? 2 : 1
  }
  return width
}

/**
 * 分析单条提交消息（可传 Buffer 或字符串）。
 *
 * 返回 `{ subject, exempt, ok, problems }`；`problems[].code` 为稳定标识，
 * 便于测试与 CI 注解复用。
 */
export function analyzeCommitMessage(input) {
  const raw = Buffer.isBuffer(input) ? input : Buffer.from(String(input ?? ''), 'utf8')
  const problems = []
  const add = (code, message) => problems.push({ code, message })

  const hasLeadingBom =
    raw.length >= BOM_BYTES.length &&
    raw[0] === BOM_BYTES[0] &&
    raw[1] === BOM_BYTES[1] &&
    raw[2] === BOM_BYTES[2]

  if (hasLeadingBom) {
    add(
      'BOM',
      '消息以 UTF-8 BOM（EF BB BF）开头，类型前缀会变成 "\\uFEFFfeat" 这类形态（多为 PowerShell 5.1 的 Set-Content -Encoding UTF8 所致）',
    )
  }

  const text = raw.toString('utf8')

  if (text.trim() === '') {
    add('EMPTY_MESSAGE', '提交消息为空')
    return { subject: '', exempt: false, ok: false, problems }
  }

  if (text.includes('\r')) {
    add('CRLF', '消息含 CR 字符（提交消息应为 LF 换行）')
  }

  const lines = text.split('\n')
  const bomLines = []
  const trailingLines = []
  lines.forEach((line, index) => {
    if (index > 0 && line.charCodeAt(0) === 0xfeff) bomLines.push(index + 1)
    if (/[ \t]+$/.test(line)) trailingLines.push(index + 1)
  })
  if (bomLines.length > 0) add('BOM', `第 ${bomLines.join('、')} 行行首含 BOM`)
  if (trailingLines.length > 0) {
    add('TRAILING_WHITESPACE', `第 ${trailingLines.join('、')} 行行尾含空白`)
  }

  // 结构检查前先剥掉 BOM，避免同一根因被重复报成「前缀不合规」。
  const subject = (lines[0] ?? '').replaceAll(BOM_CHAR, '')
  const exempt = EXEMPT_SUBJECT_PATTERN.test(subject)

  if (subject === '') {
    add('EMPTY_MESSAGE', '首行为空，应为 `type(scope): 描述`')
  } else if (!exempt) {
    if (!SUBJECT_PATTERN.test(subject)) {
      add(
        'SUBJECT_PATTERN',
        `首行不符合 \`type(scope): 描述\` —— ${JSON.stringify(subject)}（允许类型：${COMMIT_TYPES.join(' / ')}）`,
      )
    }
    const width = displayWidth(subject)
    if (width > MAX_SUBJECT_WIDTH) {
      add('SUBJECT_WIDTH', `首行显示宽度 ${width} 超过 ${MAX_SUBJECT_WIDTH}`)
    }
    if (/^\s/.test(subject)) {
      add('SUBJECT_LEADING_WHITESPACE', '首行以空白开头')
    }
  }

  if (lines.length > 1 && lines[1] !== '') {
    add('BODY_SEPARATOR', '首行与正文之间必须留一个空行')
  }

  return { subject, exempt, ok: problems.length === 0, problems }
}

function gitText(args) {
  return execFileSync('git', args, {
    encoding: 'utf8',
    maxBuffer: MAX_BUFFER,
    windowsHide: true,
  })
}

function gitBuffer(args) {
  return execFileSync('git', args, {
    maxBuffer: MAX_BUFFER,
    windowsHide: true,
  })
}

/** `core.commentChar`（默认 `#`）。hook 收到的 COMMIT_EDITMSG 含被注释的模板行。 */
function commentChar() {
  try {
    return gitText(['config', '--get', 'core.commentChar']).trim().charAt(0) || '#'
  } catch {
    return '#'
  }
}

/** 剥离注释行。刻意在文本层操作：原样保留首个 BOM，使 BOM 检查仍然生效。 */
function stripCommentLines(raw) {
  const marker = commentChar()
  const kept = raw
    .toString('utf8')
    .split('\n')
    .filter((line) => !(marker !== '' && line.startsWith(marker)))
  return Buffer.from(kept.join('\n'), 'utf8')
}

/**
 * 读取提交范围内的每个提交，返回 `{ id, sha, authorEmail, raw }`。
 *
 * 用 `git cat-file commit` 取**原始字节**，而不是 `git log --format=%B`
 * 的渲染结果 —— 后者经过编码转换，BOM 可能被静默吞掉。
 */
export function collectEntriesFromRange(range, { ignoreAuthors = new Set() } = {}) {
  const shas = gitText(['log', '--format=%H', range])
    .trim()
    .split('\n')
    .filter(Boolean)

  return shas
    .map((sha) => {
      const object = gitBuffer(['cat-file', 'commit', sha])
      const separator = object.indexOf('\n\n')
      if (separator === -1) throw new Error(`提交对象结构异常：${sha}`)
      const header = object.subarray(0, separator).toString('utf8')
      const authorEmail = /^author .*<([^>]*)>/m.exec(header)?.[1] ?? ''
      return { id: sha.slice(0, 7), sha, authorEmail, raw: object.subarray(separator + 2) }
    })
    .filter((entry) => !ignoreAuthors.has(entry.authorEmail.toLowerCase()))
}

function escapeAnnotation(text) {
  return String(text)
    .replaceAll('%', '%25')
    .replaceAll('\r', '%0D')
    .replaceAll('\n', '%0A')
}

function parseArgs(argv) {
  const options = { mode: null, target: null, range: null, ignoreAuthors: [], json: false, help: false }

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--range') {
      options.mode = 'range'
      options.range = argv[(index += 1)]
    } else if (arg === '--file') {
      options.mode = 'file'
      options.target = argv[(index += 1)]
    } else if (arg === '--stdin') {
      options.mode = 'stdin'
    } else if (arg === '--ignore-author') {
      options.ignoreAuthors.push(String(argv[(index += 1)] ?? '').toLowerCase())
    } else if (arg === '--json') {
      options.json = true
    } else if (arg === '--help' || arg === '-h') {
      options.help = true
    } else if (!arg.startsWith('-') && options.mode === null) {
      options.mode = 'file'
      options.target = arg
    } else {
      options.unknown = arg
    }
  }

  return options
}

function main() {
  const options = parseArgs(process.argv.slice(2))

  if (options.help) {
    console.log(USAGE)
    return 0
  }
  if (options.unknown) {
    console.error(`未知参数：${options.unknown}\n\n${USAGE}`)
    return 2
  }
  if (options.mode === null) {
    console.error(USAGE)
    return 2
  }

  let entries
  try {
    if (options.mode === 'range') {
      if (!options.range) throw new Error('--range 需要一个 revision range')
      entries = collectEntriesFromRange(options.range, {
        ignoreAuthors: new Set(options.ignoreAuthors.filter(Boolean)),
      })
    } else if (options.mode === 'stdin') {
      entries = [{ id: '<stdin>', raw: fs.readFileSync(0) }]
    } else {
      if (!options.target) throw new Error('--file 需要一个路径')
      entries = [{ id: path.basename(options.target), raw: fs.readFileSync(options.target) }]
    }
  } catch (error) {
    console.error(`无法读取待检查的提交消息：${error instanceof Error ? error.message : error}`)
    return 1
  }

  const results = entries.map((entry) => ({
    entry,
    result: analyzeCommitMessage(options.mode === 'range' ? entry.raw : stripCommentLines(entry.raw)),
  }))
  const failed = results.filter(({ result }) => !result.ok)

  if (options.json) {
    console.log(
      JSON.stringify(
        {
          checked: results.length,
          failed: failed.length,
          results: results.map(({ entry, result }) => ({
            id: entry.id,
            subject: result.subject,
            exempt: result.exempt,
            ok: result.ok,
            problems: result.problems,
          })),
        },
        null,
        2,
      ),
    )
    return failed.length === 0 ? 0 : 1
  }

  if (results.length === 0) {
    console.log('没有需要检查的提交（范围为空）。')
    return 0
  }

  const inActions = process.env.GITHUB_ACTIONS === 'true'

  for (const { entry, result } of results) {
    console.log(`  ${result.ok ? '✅' : '❌'} ${entry.id}  ${result.subject || '(空消息)'}`)
    for (const problem of result.problems) {
      console.log(`       · [${problem.code}] ${problem.message}`)
      if (inActions) {
        console.log(`::error title=提交消息不合规 (${problem.code})::${escapeAnnotation(`${entry.id} ${problem.message}`)}`)
      }
    }
  }

  console.log('')
  if (failed.length === 0) {
    console.log(`提交消息检查通过：${results.length} 个提交全部合规。`)
    return 0
  }

  console.log(`提交消息检查失败：${failed.length} / ${results.length} 个提交不合规。`)
  console.log('规范：`type(scope): 中文描述`（类型见 scripts/check-commit-msg.mjs 顶部注释）；')
  console.log('消息必须无 BOM、无 CRLF、无行尾空白。详见 docs-fork/agents/pi-development.md §10。')
  return 1
}

const invokedDirectly =
  typeof process.argv[1] === 'string' && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)

if (invokedDirectly) {
  process.exitCode = main()
}
