import type { FinalizationResult, ProjectSessionContext } from '../shared/ipc-channels'
import { getActiveProjectSessionContext, sameProjectSessionContext } from '../shared/project-session-context'
import type { FinalizationSnapshot } from './finalization-snapshot'
import { ipc } from './ipc-client'

/**
 * 定稿专用封装（绕过 ipc-client 的会话自动注入）。
 *
 * 迁移说明：底层由 Electron `window.velaAPI.invoke` 改为 Tauri `ipc.invoke`
 * （`@tauri-apps/api/core`）；`projectSession` 仍由本模块**显式**作为尾参传入，
 * 与基线 controller 的 `(snapshot, context)` 签名保持一致。
 */

/** 只接受已冻结快照；调用处不能把任意正文/目标路径塞给 retry。 */
export async function commitFinalizationSnapshot(
  snapshot: FinalizationSnapshot,
): Promise<FinalizationResult> {
  const currentSession = getActiveProjectSessionContext()
  if (!sameProjectSessionContext(snapshot.projectSession, currentSession)) {
    throw new Error('项目会话已变化，已拒绝提交旧定稿快照')
  }
  return ipc.invoke('finalization:commit', snapshot, snapshot.projectSession)
}

/** 实体稿重试只带已提交的 finalizationId，正文和路径始终从 SQLite outbox 读取。 */
export async function retryFinalizationPublication(
  finalizationId: string,
  projectSession: ProjectSessionContext,
): Promise<FinalizationResult> {
  const currentSession = getActiveProjectSessionContext()
  if (!sameProjectSessionContext(projectSession, currentSession)) {
    throw new Error('项目会话已变化，已拒绝实体稿重试')
  }
  return ipc.invoke('finalization:retry', finalizationId, projectSession)
}
