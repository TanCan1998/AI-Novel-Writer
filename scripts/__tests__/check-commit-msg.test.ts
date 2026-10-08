import { spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { afterEach, describe, expect, it } from 'vitest'

const checker = resolve(process.cwd(), 'scripts/check-commit-msg.mjs')

const tempRoots: string[] = []

afterEach(() => {
  for (const root of tempRoots.splice(0)) {
    rmSync(root, { recursive: true, force: true })
  }
})

function makeTempRoot(): string {
  const root = mkdtempSync(join(tmpdir(), 'check-commit-msg-'))
  tempRoots.push(root)
  return root
}

function runChecker(
  args: string[],
  options: { cwd?: string } = {},
): { status: number; stdout: string; stderr: string } {
  const result = spawnSync(process.execPath, [checker, ...args], {
    cwd: options.cwd ?? process.cwd(),
    encoding: 'utf8',
    windowsHide: true,
  })
  if (result.error) throw result.error
  return { status: result.status ?? -1, stdout: result.stdout ?? '', stderr: result.stderr ?? '' }
}

/** 写一个消息文件；传 Buffer 时可注入 BOM，传 string 时按 UTF-8 写入（无 BOM）。 */
function writeMessageFile(content: string | Buffer): string {
  const file = join(makeTempRoot(), 'COMMIT_EDITMSG')
  writeFileSync(file, Buffer.isBuffer(content) ? content : Buffer.from(content, 'utf8'))
  return file
}

const withBom = (text: string): Buffer =>
  Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from(text, 'utf8')])

function initRepo(): { root: string; git: (...args: string[]) => void } {
  const root = makeTempRoot()
  const run = (args: string[], env?: Record<string, string>) => {
    const result = spawnSync('git', args, {
      cwd: root,
      encoding: 'utf8',
      windowsHide: true,
      env: env ? { ...process.env, ...env } : process.env,
    })
    if (result.error) throw result.error
    if (result.status !== 0) {
      throw new Error(`git ${args.join(' ')} 失败：${String(result.stderr).trim()}`)
    }
  }
  run(['init', '-q', '-b', 'main'])
  run(['config', 'user.email', 'agent@example.com'])
  run(['config', 'user.name', 'Agent'])
  run(['config', 'commit.gpgsign', 'false'])
  return { root, git: (...args: string[]) => run(args) }
}

/** 提交一个空变更，消息取自 Buffer（可含 BOM）。 */
function commitWith(repo: { root: string; git: (...args: string[]) => void }, message: Buffer): void {
  const file = join(repo.root, 'note.txt')
  writeFileSync(file, `${Math.random()}\n`, 'utf8')
  repo.git('add', 'note.txt')
  const msgFile = join(repo.root, 'COMMIT_MSG_INPUT')
  writeFileSync(msgFile, message)
  repo.git('commit', '-q', '--no-verify', '-F', msgFile)
}

describe('提交消息检查（本地 hook 模式）', () => {
  it('接受本仓库规范的 type(scope): 中文描述', () => {
    const file = writeMessageFile('feat(tauri): 接入 tauri-plugin-dialog\n\n- 说明\n')
    const result = runChecker([file])
    expect(result.status).toBe(0)
    expect(result.stdout).toContain('全部合规')
  })

  it('拒绝以 UTF-8 BOM 开头的消息（2026-10-08 事故的根因）', () => {
    const file = writeMessageFile(withBom('feat(tauri): 批次 D2-c\n\n- 说明\n'))
    const result = runChecker([file])
    expect(result.status).toBe(1)
    expect(result.stdout).toContain('[BOM]')
    // 首字节 BOM 已足以定位根因，不应再叠加「前缀不合规」的噪音
    expect(result.stdout).not.toContain('[SUBJECT_PATTERN]')
  })

  it('拒绝不符合 type(scope) 前缀的首行', () => {
    const file = writeMessageFile('Update README_en.md\n')
    const result = runChecker([file])
    expect(result.status).toBe(1)
    expect(result.stdout).toContain('[SUBJECT_PATTERN]')
  })

  it('豁免 Merge / Revert / fixup! 首行，但编码问题仍然拦截', () => {
    const merge = writeMessageFile("Merge pull request #300 from x/y\n\n正文\n")
    expect(runChecker([merge]).status).toBe(0)

    const revert = writeMessageFile('Revert "feat(tauri): 某功能"\n')
    expect(runChecker([revert]).status).toBe(0)

    const dirtyMerge = writeMessageFile(withBom("Merge branch 'master' into x\n"))
    expect(runChecker([dirtyMerge]).status).toBe(1)
  })

  it('拒绝 CRLF、行尾空白与缺失的正文空行', () => {
    expect(runChecker([writeMessageFile('feat(tauri): 甲\r\n\r\n- 乙\r\n')]).stdout).toContain('[CRLF]')
    expect(runChecker([writeMessageFile('feat(tauri): 甲 \n')]).stdout).toContain('[TRAILING_WHITESPACE]')
    expect(runChecker([writeMessageFile('feat(tauri): 甲\n- 直接跟正文\n')]).stdout).toContain(
      '[BODY_SEPARATOR]',
    )
  })

  it('拒绝空消息，并检出正文行行首的 BOM', () => {
    expect(runChecker([writeMessageFile('\n\n')]).stdout).toContain('[EMPTY_MESSAGE]')
    expect(runChecker([writeMessageFile('\uFEFFfeat(tauri): 甲\n\n\uFEFF- 乙\n')]).stdout).toContain('[BOM]')
  })

  it('按显示宽度限制首行长度（CJK 计 2 列）', () => {
    const long = `feat(tauri): ${'字'.repeat(70)}`
    expect(runChecker([writeMessageFile(`${long}\n`)]).stdout).toContain('[SUBJECT_WIDTH]')
  })

  it('剥离 hook 写入的注释行，避免模板文字被误判', () => {
    const file = writeMessageFile(
      ['feat(tauri): 合法首行', '', '# Please enter the commit message for your changes.', '# 注释行', ''].join(
        '\n',
      ),
    )
    expect(runChecker([file]).status).toBe(0)
  })

  it('--json 输出机器可读结果', () => {
    const result = runChecker(['--json', writeMessageFile('Update README_en.md\n')])
    expect(result.status).toBe(1)
    const payload = JSON.parse(result.stdout) as {
      checked: number
      failed: number
      results: Array<{ id: string; ok: boolean; problems: Array<{ code: string }> }>
    }
    expect(payload.checked).toBe(1)
    expect(payload.failed).toBe(1)
    expect(payload.results[0]?.problems.map((problem) => problem.code)).toContain('SUBJECT_PATTERN')
  })

  it('用法错误返回 2，--help 返回 0', () => {
    expect(runChecker([]).status).toBe(2)
    expect(runChecker(['--bogus']).status).toBe(2)
    expect(runChecker(['--help']).status).toBe(0)
  })
})

describe('提交消息检查（CI --range 模式）', () => {
  it('逐提交读取原始字节，检出已入库的 BOM 提交', () => {
    const repo = initRepo()
    commitWith(repo, Buffer.from('feat(tauri): 干净提交\n', 'utf8'))
    commitWith(repo, withBom('feat(tauri): 带 BOM 的提交\n'))

    const result = runChecker(['--range', 'HEAD'], { cwd: repo.root })
    expect(result.status).toBe(1)
    expect(result.stdout).toContain('[BOM]')
  })

  it('通过范围内全部合规的提交', () => {
    const repo = initRepo()
    commitWith(repo, Buffer.from('feat(tauri): 甲\n', 'utf8'))
    commitWith(repo, Buffer.from('docs(tauri): 乙\n', 'utf8'))

    // `--range HEAD` 列出仓库全部可达提交（此处正好是这 2 个）
    const result = runChecker(['--range', 'HEAD'], { cwd: repo.root })
    expect(result.status).toBe(0)
    expect(result.stdout).toContain('2 个提交全部合规')
  })

  it('PR 场景：base..head 只检查新增提交，不误报 base 里的上游历史', () => {
    const repo = initRepo()
    // base 分支上模拟一条上游式、不符合本仓库规范的历史提交
    commitWith(repo, Buffer.from('Update README.md\n', 'utf8'))
    repo.git('checkout', '-q', '-b', 'feature')
    commitWith(repo, Buffer.from('feat(tauri): 新增功能\n', 'utf8'))

    const result = runChecker(['--range', 'main..HEAD'], { cwd: repo.root })
    expect(result.status).toBe(0)
    expect(result.stdout).toContain('1 个提交全部合规')
  })

  it('--ignore-author 跳过上游带入的历史提交，避免误报', () => {
    const repo = initRepo()
    commitWith(repo, Buffer.from('feat(tauri): 合规\n', 'utf8'))
    spawnSync('git', ['-c', 'user.email=upstream@example.com', 'commit', '-q', '--allow-empty', '-m', 'Update README_en.md'], {
      cwd: repo.root,
      encoding: 'utf8',
      windowsHide: true,
    })

    expect(runChecker(['--range', 'HEAD~1..HEAD'], { cwd: repo.root }).status).toBe(1)
    expect(
      runChecker(['--range', 'HEAD~1..HEAD', '--ignore-author', 'upstream@example.com'], { cwd: repo.root }).status,
    ).toBe(0)
  })

  it('范围为空时通过（base 与 head 相同）', () => {
    const repo = initRepo()
    commitWith(repo, Buffer.from('feat(tauri): 甲\n', 'utf8'))
    const result = runChecker(['--range', 'HEAD..HEAD'], { cwd: repo.root })
    expect(result.status).toBe(0)
    expect(result.stdout).toContain('没有需要检查的提交')
  })
})
