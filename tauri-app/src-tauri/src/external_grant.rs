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
//! | `resolve`（校验操作集合 + 扣减 `usesRemaining`；归零后保留记录、再消费报「已用尽」） | [`ExternalGrantRegistry::resolve`] |
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

/// 导出目录授权 TTL（对齐基线 `EXPORT_GRANT_TTL_MS = 10 分钟`）
pub const EXPORT_GRANT_TTL: Duration = Duration::from_secs(10 * 60);
/// 导出目录授权最大使用次数（对齐基线 `EXPORT_GRANT_MAX_USES = 4096`）
pub const EXPORT_GRANT_MAX_USES: u32 = 4096;

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

/// 已解析的授权目标（仅在后端内部流通，绝不序列化回渲染层）
///
/// `root` 是授权根本身（文件授权时为该文件），`path` 是拼接相对路径后的最终目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantedTarget {
    pub scope: GrantScope,
    pub root: PathBuf,
    pub path: PathBuf,
}

/// 相对路径安全校验（对齐基线 `assertSafeRelativePath`）
///
/// 拒绝：NUL、绝对路径（POSIX `/`、Windows 根分隔符、盘符、UNC）、`..` 父目录遍历。
/// 返回通过校验的原字符串（归一化由调用方完成）。
pub fn assert_safe_relative_path(relative_path: &str) -> Result<(), String> {
    if relative_path.contains('\0') {
        return Err("外部文件授权的相对路径无效".to_string());
    }
    let bytes = relative_path.as_bytes();
    let drive_prefixed = bytes.len() >= 2
        && bytes[1] == b':'
        && (bytes[0] as char).is_ascii_alphabetic();
    if relative_path.starts_with('/')
        || relative_path.starts_with('\\')
        || drive_prefixed
    {
        return Err("外部文件授权的相对路径不能是绝对路径".to_string());
    }
    if relative_path
        .split(['\\', '/'])
        .any(|segment| segment == "..")
    {
        return Err("外部文件授权的相对路径不得包含父目录遍历".to_string());
    }
    Ok(())
}

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
        Ok(self.issue(
            GrantScope::File,
            canonical,
            vec![GrantOperation::Read],
            ttl,
            max_uses,
        ))
    }

    /// 签发目录授权（路径必须已存在且为目录；默认操作 = 枚举 + 读取）
    pub fn issue_directory(
        &mut self,
        directory_path: &Path,
        ttl: Duration,
        max_uses: Option<u32>,
    ) -> Result<String, String> {
        self.issue_directory_with_operations(
            directory_path,
            vec![GrantOperation::List, GrantOperation::Read],
            ttl,
            max_uses,
        )
    }

    /// 签发带指定操作集合的目录授权（批次 H 导出目录：`write` + `create`）
    pub fn issue_directory_with_operations(
        &mut self,
        directory_path: &Path,
        operations: Vec<GrantOperation>,
        ttl: Duration,
        max_uses: Option<u32>,
    ) -> Result<String, String> {
        let canonical = canonical_existing(directory_path)?;
        if !canonical.is_dir() {
            return Err("外部文件授权目录必须是已存在的目录".to_string());
        }
        Ok(self.issue(GrantScope::Directory, canonical, operations, ttl, max_uses))
    }

    fn issue(
        &mut self,
        scope: GrantScope,
        path: PathBuf,
        operations: Vec<GrantOperation>,
        ttl: Duration,
        max_uses: Option<u32>,
    ) -> String {
        let grant_id = crate::project_access::random_uuid_v4();
        self.grants.insert(
            grant_id.clone(),
            GrantRecord {
                scope,
                path,
                operations,
                expires_at: Instant::now() + ttl,
                uses_remaining: max_uses,
            },
        );
        grant_id
    }

    /// 解析授权目标：先做纯相对路径校验（不消耗配额），再消费/只读校验授权，
    /// 最后拼接并做**词法 + 真实路径**双重容器校验（防 `..` 与 junction/symlink 逃逸）。
    pub fn resolve_target(
        &mut self,
        grant_id: &str,
        operation: GrantOperation,
        relative_path: Option<&str>,
        consume: bool,
    ) -> Result<GrantedTarget, String> {
        let requested = relative_path.unwrap_or("");
        if !matches!(self.grants.get(grant_id).map(|record| record.scope), Some(GrantScope::File)) {
            assert_safe_relative_path(requested)?;
        }

        let granted = if consume {
            self.resolve(grant_id, operation)?
        } else {
            self.revalidate(grant_id, operation)?
        };

        match granted.scope {
            GrantScope::File => {
                if !requested.is_empty() && requested != "." {
                    return Err("精确文件授权不接受子路径".to_string());
                }
                Ok(GrantedTarget {
                    scope: granted.scope,
                    root: granted.path.clone(),
                    path: granted.path,
                })
            }
            GrantScope::Directory => {
                let joined = granted.path.join(requested);
                let target = crate::security::lexically_normalize(&joined.to_string_lossy());
                let root_lex = crate::security::lexically_normalize(&granted.path.to_string_lossy());
                if !crate::security::lexically_contained(&root_lex, &target) {
                    return Err("外部文件授权目标超出授权范围".to_string());
                }
                let root_canonical = std::fs::canonicalize(&root_lex)
                    .map_err(|error| format!("外部文件授权目录已失效（{error}）"))?;
                match crate::security::canonical_writable_target(&target) {
                    Some(canonical) if crate::security::lexically_contained(&root_canonical, &canonical) => {}
                    Some(_) => return Err("外部文件授权目标超出授权范围".to_string()),
                    None => return Err("外部文件授权目标无效".to_string()),
                }
                Ok(GrantedTarget {
                    scope: granted.scope,
                    root: granted.path,
                    path: target,
                })
            }
        }
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

        // 消费：扣减次数；归零后**保留记录**（对齐基线 `resolveRequest`：
        // `revalidate` 仍可用，仅后续消费型调用报「已用尽」），过期才删除。
        // 该语义是 `fs:grant-write-file` 的前提：消费 `write` 之后还需
        // `revalidate('create')` 探测与 ready 阶段复检。
        if consume {
            let record = self
                .grants
                .get_mut(grant_id)
                .ok_or_else(|| INVALID_GRANT_MESSAGE.to_string())?;
            match record.uses_remaining {
                Some(0) => return Err("外部文件授权已用尽".to_string()),
                Some(remaining) => record.uses_remaining = Some(remaining - 1),
                None => {}
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
        assert_eq!(registry.len(), 1, "用尽后保留记录（对齐基线），过期才删除");

        // 再解析报「已用尽」；但 revalidate 仍可用（记录尚在）
        assert_eq!(
            registry.resolve(&grant_id, GrantOperation::Read).unwrap_err(),
            "外部文件授权已用尽"
        );
        assert!(registry.revalidate(&grant_id, GrantOperation::Read).is_ok());
        assert_eq!(registry.len(), 1);

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
        // list 消费（用尽后记录保留，后续消费型调用报「已用尽」）
        assert!(registry.resolve(&grant_id, GrantOperation::List).is_ok());
        assert_eq!(registry.len(), 1, "用尽后保留记录（对齐基线）");
        assert_eq!(
            registry.resolve(&grant_id, GrantOperation::List).unwrap_err(),
            "外部文件授权已用尽"
        );

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

    #[test]
    fn export_directory_grant_allows_write_and_create_only_test() {
        let dir = temp_dir("export-ops");
        let mut registry = ExternalGrantRegistry::default();
        let grant_id = registry
            .issue_directory_with_operations(
                &dir,
                vec![GrantOperation::Write, GrantOperation::Create],
                EXPORT_GRANT_TTL,
                Some(EXPORT_GRANT_MAX_USES),
            )
            .unwrap();

        // 导出授权不得读取/枚举
        assert!(registry.revalidate(&grant_id, GrantOperation::Read).is_err());
        assert!(registry.revalidate(&grant_id, GrantOperation::List).is_err());
        // 写入与创建均可用，且 revalidate 不消耗
        assert!(registry.revalidate(&grant_id, GrantOperation::Write).is_ok());
        assert!(registry.revalidate(&grant_id, GrantOperation::Create).is_ok());
        assert_eq!(registry.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_target_validates_relative_paths_test() {
        let dir = temp_dir("relative");
        std::fs::create_dir_all(dir.join("sub")).unwrap();

        // 纯函数校验
        assert!(assert_safe_relative_path("a.txt").is_ok());
        assert!(assert_safe_relative_path("sub/a.txt").is_ok());
        assert_eq!(
            assert_safe_relative_path("/abs.txt").unwrap_err(),
            "外部文件授权的相对路径不能是绝对路径"
        );
        assert_eq!(
            assert_safe_relative_path("C:\\abs.txt").unwrap_err(),
            "外部文件授权的相对路径不能是绝对路径"
        );
        assert_eq!(
            assert_safe_relative_path("a\\..\\b.txt").unwrap_err(),
            "外部文件授权的相对路径不得包含父目录遍历"
        );
        assert_eq!(
            assert_safe_relative_path("a\0b").unwrap_err(),
            "外部文件授权的相对路径无效"
        );

        // 目录授权：相对路径解析到授权根内
        let mut registry = ExternalGrantRegistry::default();
        let grant_id = registry
            .issue_directory(&dir, KNOWLEDGE_BASE_GRANT_TTL, None)
            .unwrap();
        let target = registry
            .resolve_target(&grant_id, GrantOperation::Read, Some("sub/a.txt"), true)
            .unwrap();
        assert_eq!(target.path.file_name().unwrap().to_string_lossy(), "a.txt");
        assert_eq!(
            std::fs::canonicalize(target.path.parent().unwrap()).unwrap(),
            std::fs::canonicalize(dir.join("sub")).unwrap()
        );
        assert_eq!(target.root, std::fs::canonicalize(&dir).unwrap());

        // 路径越界被拒（词法层）
        assert_eq!(
            registry
                .resolve_target(&grant_id, GrantOperation::Read, Some("../escape.txt"), true)
                .unwrap_err(),
            "外部文件授权的相对路径不得包含父目录遍历"
        );

        // 文件授权：不接受子路径
        let file = dir.join("b.txt");
        std::fs::write(&file, "x").unwrap();
        let file_grant = registry
            .issue_file(&file, KNOWLEDGE_BASE_GRANT_TTL, None)
            .unwrap();
        assert_eq!(
            registry
                .resolve_target(&file_grant, GrantOperation::Read, Some("c.txt"), true)
                .unwrap_err(),
            "精确文件授权不接受子路径"
        );
        let ok = registry
            .resolve_target(&file_grant, GrantOperation::Read, None, true)
            .unwrap();
        assert_eq!(ok.path, std::fs::canonicalize(&file).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
