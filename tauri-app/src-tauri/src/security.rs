//! 项目文件系统安全边界 —— 迁移自 `electron/utils/project-context.ts` +
//! `electron/services/project-access.ts`。
//!
//! 纪律（ADR 0001 会话租约 / ADR 0002 外部文件授权）：
//! - 每次触碰文件系统前都以调用时冻结的完整会话重新认证，不能只比较路径；
//! - 路径边界三层：expectedProjectPath 匹配 → 词法包含检查 → realpath 防
//!   symlink/junction 指向项目外部；
//! - 租约校验已接入真实会话（批次 C）：`projectId`、`leaseId`、`projectPath`
//!   三字段必须同时匹配活跃项目，否则一律拒绝。

use std::path::{Component, Path, PathBuf};

/// 项目会话上下文 —— 迁移自 `src/shared/project-session-context.ts` 的
/// `ProjectSessionContext`（`isProjectSessionContext` 三字段判别语义）。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSessionContext {
    pub project_id: String,
    pub lease_id: String,
    pub project_path: String,
}

/// 守卫拒绝原因 —— 对齐 `fs-controller.ts` 的错误分类（文案见 [`guard_message`]，
/// 骨架阶段直接输出中文，TODO i18n 随批次 E 收口）。
///
/// `MissingSessionContext` 在当前命令签名下由 Tauri 必填参数解析先行拦下，
/// 保留该态供内部调用链使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardKind {
    /// 缺少项目会话上下文（渲染层未注入 `projectSession`）。
    /// Tauri 必填参数解析先行拦下缺失会话，仅在 `Option` 尾参为空时手工构造；
    /// 保留该态以对齐基线文案，暂抑制死代码警告。
    #[allow(dead_code)]
    MissingSessionContext,
    /// 目标超出当前项目范围（词法或 realpath 检查失败）。
    OutsideProject,
    /// 跨项目读写（expectedProjectPath 与活跃项目不一致）。
    CrossProject,
    /// 租约失效（无活跃项目、会话字段不完整，或租约与活跃会话不符）。
    LeaseInvalid,
}

pub type GuardResult = Result<(), GuardKind>;

/// 对齐 `projectFilesystemFailure` 的 accessMessage 文案（中文基线）。
pub fn guard_message(kind: GuardKind) -> String {
    match kind {
        GuardKind::MissingSessionContext => "缺少项目会话上下文，已拒绝操作。".to_string(),
        GuardKind::OutsideProject => "目标超出当前项目范围，已拒绝操作。".to_string(),
        GuardKind::CrossProject => "检测到跨项目读写，已拒绝操作。".to_string(),
        GuardKind::LeaseInvalid => "项目租约已失效，已拒绝操作。".to_string(),
    }
}

/// 路径断言模式 —— existing 适用于读取/枚举；writable 允许目标尚未存在。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathCheckMode {
    Existing,
    Writable,
}

/// 词法归一化：统一分隔符、消 `.`/中间 `..`/尾分隔符并转绝对路径。
/// 对应 Electron `path.resolve` 的词法部分（不触文件系统）。
pub fn lexically_normalize(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return PathBuf::new();
    }
    let mut out = PathBuf::new();
    // Windows 上 absolute() 只做词法化（不加 \\?\ 设备前缀），与 Node path.resolve
    // 的词法行为对齐；失败时按相对路径归一处理。
    let base = std::path::absolute(trimmed).unwrap_or_else(|_| PathBuf::from(trimmed));
    for comp in base.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                // 绝对路径下 pop 总是可行；相对根上的 `..` 骨架阶段直接忽略
                //（IPC 传入的均为绝对项目内路径）。
                let _ = out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// normalizedProjectPath：resolve + en-US 小写，用于跨项目/会话路径一致性比较。
pub fn normalized_project_path(path: &str) -> String {
    lexically_normalize(path).to_string_lossy().to_lowercase()
}

/// 词法包含检查：target 必须等于 root 或位于 root 之内。
/// 输入应为已词法归一化的路径（`..` 段已被消除）。
pub fn lexically_contained(root: &Path, target: &Path) -> bool {
    target.strip_prefix(root).is_ok()
}

/// canonicalWritableTarget：目标可不存在——向上找最近的现有祖先 realpath，
/// 再拼回缺失段（迁移自 electron/utils/project-context.ts 同名函数）。
pub fn canonical_writable_target(path: &Path) -> Option<PathBuf> {
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut existing = path.to_path_buf();
    loop {
        if existing.is_dir() || existing.is_file() {
            break;
        }
        match existing.parent() {
            Some(parent) if parent != existing => {
                missing.push(existing.file_name()?.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => return None, // 到达根仍不存在（不可能的 fs 状态）
        }
    }
    let canonical = std::fs::canonicalize(&existing).ok()?;
    let mut result = canonical;
    for seg in missing.into_iter().rev() {
        result.push(seg);
    }
    Some(result)
}

/// assertProjectFilePath：词法边界 + realpath 双重检查，拒绝 junction/symlink
/// 指向项目外部。先做词法检查避免对越界路径暴露 ENOENT。
pub fn assert_project_file_path(
    file_path: &str,
    root: &str,
    mode: PathCheckMode,
) -> Result<PathBuf, GuardKind> {
    let root_lex = lexically_normalize(root);
    let target_lex = lexically_normalize(file_path);
    // 先词法边界检查，避免对明显越界且不存在的路径暴露 ENOENT。
    if !lexically_contained(&root_lex, &target_lex) {
        return Err(GuardKind::OutsideProject);
    }
    let root_canonical = std::fs::canonicalize(&root_lex).map_err(|_| GuardKind::OutsideProject)?;
    let target_canonical = match mode {
        PathCheckMode::Existing => std::fs::canonicalize(&target_lex).ok(),
        PathCheckMode::Writable => canonical_writable_target(&target_lex),
    }
    .ok_or(GuardKind::OutsideProject)?;
    if !lexically_contained(&root_canonical, &target_canonical) {
        return Err(GuardKind::OutsideProject);
    }
    Ok(target_canonical)
}

/// 会话租约校验（批次 C 已接入真实租约）：
///
/// 会话三字段非空 + `projectId`/`leaseId` 与活跃会话完全一致 + 会话项目路径
/// 与活跃项目根一致。路径从不单独构成授权。
pub fn assert_current_project_context(
    context: &ProjectSessionContext,
    active: Option<&crate::state::ActiveProject>,
) -> GuardResult {
    if context.project_id.trim().is_empty()
        || context.lease_id.trim().is_empty()
        || context.project_path.trim().is_empty()
    {
        return Err(GuardKind::LeaseInvalid);
    }
    let active = active.ok_or(GuardKind::LeaseInvalid)?;
    if active.project_id != context.project_id || active.lease_id != context.lease_id {
        return Err(GuardKind::LeaseInvalid);
    }
    if normalized_project_path(&active.root_path) != normalized_project_path(&context.project_path) {
        return Err(GuardKind::CrossProject);
    }
    Ok(())
}

/// expectedProjectPath 匹配守卫（可选版）：expected 为空则跳过。
pub fn assert_expected_project_path(
    current: Option<&str>,
    expected: Option<&str>,
) -> Result<(), GuardKind> {
    let expected = match expected {
        Some(e) if !e.trim().is_empty() => e,
        _ => return Ok(()),
    };
    match current {
        Some(current) if normalized_project_path(current) == normalized_project_path(expected) => Ok(()),
        _ => Err(GuardKind::CrossProject),
    }
}

/// 对可能修改或读取项目数据的通道，调用方必须显式携带冻结的项目身份。
pub fn assert_required_expected_project_path(
    current: Option<&str>,
    expected: Option<&str>,
) -> Result<(), GuardKind> {
    let expected = match expected {
        Some(e) if !e.trim().is_empty() => e,
        _ => return Err(GuardKind::CrossProject),
    };
    assert_expected_project_path(current, Some(expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_project_path_folds_case_and_separators() {
        let a = normalized_project_path("F:\\Workplace\\Novel");
        let b = normalized_project_path("f:/workplace/novel");
        assert_eq!(a, b);
    }

    #[test]
    fn lexically_contained_rejects_escape() {
        let root = PathBuf::from("F:/proj");
        assert!(lexically_contained(&root, &PathBuf::from("F:/proj/chapter/1.md")));
        assert!(lexically_contained(&root, &root));
        assert!(!lexically_contained(&root, &lexically_normalize("F:/proj/../outside")));
        assert!(!lexically_contained(&root, &PathBuf::from("F:/other/x.md")));
        // target 是 root 祖先 → 拒绝
        assert!(!lexically_contained(
            &PathBuf::from("F:/proj/sub"),
            &PathBuf::from("F:/proj")
        ));
    }

    #[test]
    fn lexically_normalize_removes_dot_segments() {
        let n = lexically_normalize("F:/proj/./a/../b");
        assert_eq!(n, PathBuf::from("F:/proj/b"));
    }

    #[test]
    fn required_expected_project_path_rejects_missing() {
        assert!(assert_required_expected_project_path(Some("F:/p"), None).is_err());
        assert!(assert_required_expected_project_path(Some("F:/p"), Some("  ")).is_err());
        assert!(assert_required_expected_project_path(None, Some("F:/p")).is_err());
        assert!(assert_required_expected_project_path(Some("F:/p"), Some("F:/p")).is_ok());
    }

    #[test]
    fn project_file_path_rejects_escape() {
        assert!(assert_project_file_path("F:/proj/out/a.md", "F:/proj", PathCheckMode::Existing).is_err());
    }

    #[test]
    fn canonical_writable_target_resolves_missing_segments() {
        let dir = std::env::temp_dir().join("vela-path-guard-test");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("not-yet-created").join("file.md");
        let resolved = canonical_writable_target(&target);
        assert!(resolved.is_some(), "允许目标尚未存在的写入路径解析");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
