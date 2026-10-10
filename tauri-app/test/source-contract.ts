import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

/** 本文件位于 `tauri-app/test/`，其父目录即 `tauri-app/` 根。 */
const appRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '..',
)

/**
 * Source-level contract tests intentionally ignore checkout line endings.
 * Git may materialize the same tracked source as LF or CRLF on Windows.
 */
export function normalizeSourceEol(source: string): string {
  return source.replace(/\r\n?/g, '\n')
}

/**
 * Read a tracked source file through the same canonical EOL boundary used by
 * source-contract assertions and content hashes.
 *
 * `sourcePath` 相对 `tauri-app/` 根解析（绝对路径原样返回）。
 */
export function readNormalizedSource(sourcePath: string): string {
  return normalizeSourceEol(
    readFileSync(path.resolve(appRoot, sourcePath), 'utf8'),
  )
}
