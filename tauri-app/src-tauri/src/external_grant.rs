//! 外部文件授权注册表（进程内存态）—— 平移自 `electron/services/external-file-grant-service.ts`。
//!
//! ADR 0002：外部文件必须在用户经系统对话框显式授权后才能读取。渲染进程只携带
//! **不透明 grantId**，绝不传入/持有绝对路径；路径只存在于本注册表内。
//!
//! # 与基线的对齐
//!
//! | 基线 | 本实现 |
//! |---|---|
//! | `issueFile` / `issueDirectory`（签发前 `realpathSync` + 存在性校验） | [`ExternalGrantRegistry::issue_file`] / [`ExternalGrantRegistry::issue_directory`] |
//! | `resolve`（校验操作集合 + 扣减 `usesRemaining`，用尽即删除） | [`ExternalGrantRegistry::resolve`] |
//! | `revalidate`（只校验、不消费） | [`ExternalGrantRegistry::revalidate`] |
//! | `revoke` | [`ExternalGrantRegistry::revoke`] |
//!
//! **刻意不迁移**：基线的 `webContentsId` 归属校验（Tauri 无多 webContents 语义；
//! 本项目单窗口，grant 归属由进程内注册表隔离即可）。`deleteOnConsume`/`maxUses`
//! 统一为「可选最大使用次数」。
//!
//! # 批次边界
//!
//! 本模块是批次 H `fs:grant-*` 命令与批次 F2 `dialog:select-knowledge-*` 的**共用底座**。
//! 本批（F2-3）只接入知识库选择/导入路径；批次 H 复用同一注册表，勿另建。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 知识库导入授权 TTL（对齐基线 `KNOWLEDGE_BASE_GRANT_TTL_MS = 10 分钟`）
pub const KNOWLEDGE_BASE_GRANT_TTL: Duration = Duration::from_secs(10 * 60);

/// 授权可执行的操作（对齐基线 `ExternalFileGrantOperation`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantOperation {
    Read,
    List,
    Write,
    Create,
    Show,
}

impl GrantOperation {
    /// 中文可读标签（错误文案用）
    fn label(self) -> &'static str {
        match self {
            Self::Read => "读取",
            Self::List => "枚举",
            Self::Write => "写入",
            Self::Create => "创建",
            Self::Show => "显示",
        }
    }
}

/// 授权范围
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantScope {
    File,
    Directory,
}

/// 已解析的授权目标（仅在后端内部流通，绝不序列化回渲染层）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantedPath {
    pub scope: GrantScope,
    pub path: PathBuf,
}

#[derive(Debug)]
struct GrantRecord {
    scope: GrantScope,
    path: PathBuf,
    operations: Vec<GrantOperation>,
    expires_at: Instant,
    /// `None` = 不限次数；`Some(n)` 每次 `resolve` 扣 1，归零即删除。
    uses_remaining: Option<u32>,
}

/// 外部文件授权注册表（常驻 `AppState`，进程重启即失效）
#[derive(Debug, Default)]
pub struct ExternalGrantRegistry {
    grants: HashMap<String, GrantRecord>,
}

/// 无效/过期授权的统一文案（对齐控制器 `invalidExternalGrantText` 的中文口径）
pub const INVALID_GRANT_MESSAGE: &str =
    "外部文件授权无效或已失效，请重新选择。";

impl ExternalGrantRegistry {
    /// 签发单文件授权（路径必须已存在且为文件）
    pub fn issue_file(
        &mut self,
        file_path: &Path,
        ttl: Duration,
        max_uses: Option<u32>,
    ) -> Result<String, String> {
        let canonical = canonical_existing(file_path)?;
        if !canonical.is_file() {
            return Err("外部文件授权目标必须是已存在的文件".to_string());
        }
        Ok(self.issue(GrantScope::File, canonical, ttl, max_uses))
    }

    /// 签发目录授权（路径必须已存在且为目录）
    pub fn issue_directory(
        &mut self,
        directory_path: &Path,
        ttl: Duration,
        max_uses: Option<u32>,
    ) -> Result<String, String> {
        let canonical = canonical_existing(directory_path)?;
        if !canonical.is_dir() {
            return Err("外部文件授权目录必须是已存在的目录".to_string());
        }
        Ok(self.issue(GrantScope::Directory, canonical, ttl, max_uses))
    }

    fn issue(
        &mut self,
        scope: GrantScope,
        path: PathBuf,
        ttl: Duration,
        max_uses: Option<u32>,
    ) -> String {
        let grant_id = crate::project_access::random_uuid_v4();
        self.grants.insert(
            grant_id.clone(),
            GrantRecord {
                scope,
                path,
                operations: match scope {
                    GrantScope::File => vec![GrantOperation::Read],
                    GrantScope::Directory => vec![GrantOperation::List, GrantOperation::Read],
                },
                expires_at: Instant::now() + ttl,
                uses_remaining: max_uses,
            },
        );
        grant_id
    }

    /// 校验并**消费**一次授权（用尽即删除）
    pub fn resolve(
        &mut self,
        grant_id: &str,
        operation: GrantOperation,
    ) -> Result<GrantedPath, String> {
        self.check(grant_id, operation, true)
    }

    /// 只校验、不消费（对齐基线 `revalidate`）
    pub fn revalidate(
        &mut self,
        grant_id: &str,
        operation: GrantOperation,
    ) -> Result<GrantedPath, String> {
        self.check(grant_id, operation, false)
    }

    /// 撤销授权（重复撤销无副作用）
    pub fn revoke(&mut self, grant_id: &str) {
        self.grants.remove(grant_id);
    }

    fn check(
        &mut self,
        grant_id: &str,
        operation: GrantOperation,
        consume: bool,
    ) -> Result<GrantedPath, String> {
        let (expired, granted) = {
            let record = self
                .grants
                .get(grant_id)
                .ok_or_else(|| INVALID_GRANT_MESSAGE.to_string())?;
            if Instant::now() >= record.expires_at {
                (true, None)
            } else {
                if !record.operations.contains(&operation) {
                    return Err(format!(
                        "外部文件授权不允许{}操作",
                        operation.label()
                    ));
                }
                (
                    false,
                    Some(GrantedPath {
                        scope: record.scope,
                        path: record.path.clone(),
                    }),
                )
            }
        };
        if expired {
            self.grants.remove(grant_id);
            return Err(INVALID_GRANT_MESSAGE.to_string());
        }

        // 消费：扣减次数，归零即删除（保证授权一次性语义）
        if consume {
            let exhausted = {
                let record = self
                    .grants
                    .get_mut(grant_id)
                    .ok_or_else(|| INVALID_GRANT_MESSAGE.to_string())?;
                match record.uses_remaining {
                    Some(remaining) if remaining > 1 => {
                        record.uses_remaining = Some(remaining - 1);
                        false
                    }
                    Some(_) => true,
                    None => false,
                }
            };
            if exhausted {
                self.grants.remove(grant_id);
            }
        }

        granted.ok_or_else(|| INVALID_GRANT_MESSAGE.to_string())
    }

    /// 当前存活授权数（测试/诊断用）
    pub fn len(&self) -> usize {
        self.grants.len()
    }

    /// 是否无存活授权
    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }
}

/// `realpath` 并拒绝不存在 / 非法路径
fn canonical_existing(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path)
        .map_err(|error| format!("外部文件授权路径无效（{}）：{error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorekeeper-grant-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn issue_file_then_resolve_once_then_expired_test() {
        let dir = temp_dir("file");
        let file = dir.join("a.txt");
        std::fs::write(&file, "正文").unwrap();

        let mut registry = ExternalGrantRegistry::default();
        let grant_id = registry
            .issue_file(&file, KNOWLEDGE_BASE_GRANT_TTL, Some(1))
            .unwrap();
        assert_eq!(registry.len(), 1);

        let granted = registry.resolve(&grant_id, GrantOperation::Read).unwrap();
        assert_eq!(granted.scope, GrantScope::File);
        assert_eq!(granted.path, std::fs::canonicalize(&file).unwrap());
        assert!(registry.is_empty(), "一次性授权用尽即删除");

        // 再解析应失效
        assert!(registry.resolve(&grant_id, GrantOperation::Read).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_grant_allows_list_and_read_but_not_write_test() {
        let dir = temp_dir("dir");
        let mut registry = ExternalGrantRegistry::default();
        let grant_id = registry
            .issue_directory(&dir, KNOWLEDGE_BASE_GRANT_TTL, Some(1))
            .unwrap();

        // revalidate 不消费
        assert!(registry.revalidate(&grant_id, GrantOperation::Read).is_ok());
        assert!(registry.revalidate(&grant_id, GrantOperation::Read).is_ok());
        // 目录授权不含写
        assert!(registry.resolve(&grant_id, GrantOperation::Write).is_err());
        // list 消费
        assert!(registry.resolve(&grant_id, GrantOperation::List).is_ok());
        assert!(registry.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_without_uses_is_unlimited_test() {
        let dir = temp_dir("unlimited");
        let file = dir.join("a.txt");
        std::fs::write(&file, "x").unwrap();

        let mut registry = ExternalGrantRegistry::default();
        let grant_id = registry.issue_file(&file, KNOWLEDGE_BASE_GRANT_TTL, None).unwrap();
        for _ in 0..3 {
            assert!(registry.resolve(&grant_id, GrantOperation::Read).is_ok());
        }
        assert_eq!(registry.len(), 1, "不限次数授权不因消费而删除");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn issue_rejects_missing_or_wrong_kind_test() {
        let dir = temp_dir("kind");
        let mut registry = ExternalGrantRegistry::default();
        assert!(registry.issue_file(&dir, KNOWLEDGE_BASE_GRANT_TTL, Some(1)).is_err());
        assert!(registry
            .issue_directory(&dir.join("missing"), KNOWLEDGE_BASE_GRANT_TTL, Some(1))
            .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn revoke_removes_grant_test() {
        let dir = temp_dir("revoke");
        let file = dir.join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let mut registry = ExternalGrantRegistry::default();
        let grant_id = registry.issue_file(&file, KNOWLEDGE_BASE_GRANT_TTL, Some(1)).unwrap();
        registry.revoke(&grant_id);
        assert!(registry.is_empty());
        assert!(registry.resolve(&grant_id, GrantOperation::Read).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
