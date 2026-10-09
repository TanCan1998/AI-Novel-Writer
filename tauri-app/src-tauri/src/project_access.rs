//! 项目访问与身份服务 —— 平移自 `electron/services/project-access.ts`
//!
//! 职责：项目根探测（清单 / 旧版指纹）、项目创建、清单写入、会话租约签发与校验、
//! 删除授权。**路径从不单独构成授权**：所有项目级操作都必须额外通过租约校验。
//!
//! 与基线的差异（已记录为迁移遗留）：
//! - 租约 id / 项目 id 使用进程内自实现的 UUID v4（零新依赖），基线使用 `node:crypto.randomUUID`；
//! - 基线 `realpathSync.native` 等价物为 `std::fs::canonicalize`，其 Windows 前缀
//!   （`\\?\`）在 `project_path_key` 中归一化后再比较。

use rusqlite::OpenFlags;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 项目清单相对路径（`.lore/project.json`）
pub const PROJECT_MANIFEST_DIR: &str = ".lore";
/// 项目清单文件名
pub const PROJECT_MANIFEST_FILE: &str = "project.json";
/// 「所选目录不是项目根」错误码（渲染层据此提示选择有效项目目录）
pub const PROJECT_ROOT_REQUIRED_CODE: &str = "PROJECT_ROOT_REQUIRED";
/// 「所选目录不是项目根」错误文案（探测唯一出口，供调用方精确比对）
pub const PROJECT_ROOT_REQUIRED_MESSAGE: &str =
    "所选目录不是项目根目录：目录缺少有效项目清单或可信旧版指纹";
/// 项目提示词目录相对路径（对齐 `src/shared/project-paths.ts` 的 `DIR_PROMPTS`）
pub const DIR_PROMPTS_RELATIVE: &str = ".lore/prompts";

/// 旧版项目可信指纹所需表（基线 `LEGACY_REQUIRED_TABLES`）
const LEGACY_REQUIRED_TABLES: [&str; 5] = [
    "project_core",
    "blueprints",
    "characters",
    "contents",
    "drafts",
];
/// 旧版项目 `project_core` 必需列（基线 `LEGACY_PROJECT_CORE_COLUMNS`）
const LEGACY_PROJECT_CORE_COLUMNS: [&str; 5] = [
    "id",
    "project_name",
    "genre",
    "total_chapters",
    "character_states",
];

/// 通过清单确认的可信项目
#[derive(Debug, Clone, PartialEq)]
pub struct TrustedProject {
    pub project_id: String,
    pub root_path: String,
}

/// 项目探测结果
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectProbe {
    /// 已有有效清单
    Manifest(TrustedProject),
    /// 旧版 SQLite 指纹（待收养写入清单）
    Legacy { root_path: String },
}

impl ProjectProbe {
    /// 探测到的项目根路径
    ///
    /// 当前调用点均直接匹配变体取字段；本方法供后续子域（导入/别名解析）统一取用，
    /// 暂抑制死代码警告。
    #[allow(dead_code)]
    pub fn root_path(&self) -> &str {
        match self {
            ProjectProbe::Manifest(project) => &project.root_path,
            ProjectProbe::Legacy { root_path } => root_path,
        }
    }
}

/// 项目会话租约（主进程侧登记的活动会话）
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectSessionLease {
    pub project_id: String,
    pub root_path: String,
    pub lease_id: String,
}

/// 平台无关的路径身份键：去掉 Windows 扩展前缀、统一分隔符、大小写折叠
pub fn project_path_key(project_path: &str) -> String {
    let mut value = project_path.trim().replace('/', "\\");
    if let Some(stripped) = value.strip_prefix("\\\\?\\") {
        // UNC 扩展路径还原为普通 UNC 形式，其余直接去前缀
        let lowered = stripped.to_ascii_lowercase();
        value = if let Some(_rest) = lowered.strip_prefix("unc\\") {
            format!("\\\\{}", &stripped[4..])
        } else {
            stripped.to_string()
        };
    }
    while value.len() > 1 && value.ends_with('\\') {
        value.pop();
    }
    value.to_lowercase()
}

/// 目标路径是否位于根目录之内（词法包含，按身份键比较）
pub fn is_contained_path(root_path: &str, target_path: &str) -> bool {
    let root = project_path_key(root_path);
    let target = project_path_key(target_path);
    if root.is_empty() || target.is_empty() {
        return false;
    }
    if target == root {
        return true;
    }
    let separator = if root.ends_with('\\') { "" } else { "\\" };
    target.starts_with(&format!("{root}{separator}"))
}

/// 项目名净化（基线 `sanitizeProjectName`）
pub fn sanitize_project_name(name: &str) -> String {
    let replaced: String = name
        .trim()
        .chars()
        .map(|character| {
            if matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) || (character as u32) < 32
            {
                '_'
            } else {
                character
            }
        })
        .collect();
    let sanitized = replaced.trim().to_string();
    if sanitized.is_empty() {
        "未命名项目".to_string()
    } else {
        sanitized
    }
}

/// 项目目录名净化（基线 `sanitizeProjectDirectoryName`）
pub fn sanitize_project_directory_name(name: &str) -> String {
    let trimmed = name.trim();
    let base_name = trimmed
        .rsplit(['\\', '/'])
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or(trimmed);
    let replaced: String = base_name
        .chars()
        .map(|character| {
            if matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) || (character as u32) < 32
            {
                '_'
            } else {
                character
            }
        })
        .collect();
    let sanitized = replaced.trim().to_string();
    if sanitized.is_empty() {
        "未命名项目".to_string()
    } else {
        sanitized
    }
}

/// 生成 UUID v4（零依赖实现：时间 + PID + 计数器 + ASLR 熵 经 splitmix64 混淆）
///
/// 用途是项目/租约身份标识（唯一性），非密码学用途。
pub fn random_uuid_v4() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let mut state = now
        ^ ((u64::from(std::process::id())) << 32)
        ^ COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    let stack_hint = (&state as *const u64) as u64;
    state ^= stack_hint.rotate_left(17);

    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = state;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^= mixed >> 31;
        let encoded = mixed.to_le_bytes();
        chunk.copy_from_slice(&encoded[..chunk.len()]);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 10xx

    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// 校验字符串是否为 UUID v4 形式（基线正则的等价实现）
pub fn is_uuid_v4(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        match index {
            8 | 13 | 18 | 23 => {
                if *byte != b'-' {
                    return false;
                }
            }
            _ => {
                if !byte.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    // version nibble = 4（位置 14），variant nibble ∈ {8,9,a,b}（位置 19）
    if bytes[14] != b'4' {
        return false;
    }
    matches!(bytes[19].to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b')
}

/// 校验清单内容是否为有效项目清单（基线 `isProjectManifest`）
pub fn is_project_manifest(value: &serde_json::Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.get("schemaVersion").and_then(|item| item.as_i64()) != Some(1) {
        return false;
    }
    if object.get("kind").and_then(|item| item.as_str()) != Some("ai-novel-project") {
        return false;
    }
    let Some(project_id) = object.get("projectId").and_then(|item| item.as_str()) else {
        return false;
    };
    if !is_uuid_v4(project_id) {
        return false;
    }
    match object.get("createdAt").and_then(|item| item.as_str()) {
        Some(created_at) => is_iso8601_timestamp(created_at),
        None => false,
    }
}

/// 宽松校验 ISO8601 时间戳（`YYYY-MM-DDTHH:MM:SS` 前缀 + 可解析尾部）
fn is_iso8601_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 19 {
        return false;
    }
    let digit_positions = [0usize, 1, 2, 3, 5, 6, 8, 9];
    for position in digit_positions {
        if !bytes[position].is_ascii_digit() {
            return false;
        }
    }
    bytes[4] == b'-'
        && bytes[7] == b'-'
        && matches!(bytes[10], b'T' | b't' | b' ')
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19..].iter().all(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'.' | b'Z' | b'z' | b'+' | b'-' | b':')
        })
}

/// 解析为 canonical 目录（存在性 + 目录性 + 符号链接展开）
pub fn canonical_existing_directory(candidate: &str) -> Result<String, String> {
    if candidate.trim().is_empty() {
        return Err("项目目录不能为空".to_string());
    }
    let resolved = PathBuf::from(candidate);
    if !resolved.exists() {
        return Err("项目目录不存在".to_string());
    }
    if !resolved.is_dir() {
        return Err("项目根必须是目录".to_string());
    }
    let canonical =
        std::fs::canonicalize(&resolved).map_err(|error| format!("项目目录无法解析：{error}"))?;
    Ok(normalize_canonical(canonical))
}

/// 用户主目录（用于拒绝把主目录当作项目根）
pub fn home_directory() -> Option<String> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .and_then(|value| canonical_existing_directory(&value).ok())
}

/// canonical 项目根（拒绝磁盘根与用户主目录）
pub fn canonical_project_root(candidate: &str, home: Option<&str>) -> Result<String, String> {
    let root_path = canonical_existing_directory(candidate)?;
    let key = project_path_key(&root_path);
    // 磁盘根：去掉分隔符后形如 "c:" 或 "\\server\share"
    let trimmed = key.trim_end_matches('\\').to_string();
    let is_drive_root = trimmed.len() == 2 && trimmed.ends_with(':');
    if is_drive_root || trimmed.is_empty() {
        return Err("拒绝将磁盘根目录作为项目".to_string());
    }
    if let Some(home) = home {
        if project_path_key(home) == key {
            return Err("拒绝将用户主目录作为项目".to_string());
        }
    }
    Ok(root_path)
}

/// 去掉 Windows 扩展长度前缀（`\\?\`），使路径与 Electron 侧形态一致
fn normalize_canonical(path: PathBuf) -> String {
    let text = path.to_string_lossy().to_string();
    text.strip_prefix("\\\\?\\").unwrap_or(&text).to_string()
}

/// 项目清单路径
pub fn manifest_path(root_path: &str) -> PathBuf {
    Path::new(root_path)
        .join(PROJECT_MANIFEST_DIR)
        .join(PROJECT_MANIFEST_FILE)
}

/// 写入项目清单（`wx` 语义：已存在即失败）
fn write_manifest(root_path: &str) -> Result<TrustedProject, String> {
    let manifest_file = manifest_path(root_path);
    if let Some(parent) = manifest_file.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("创建 .lore 目录失败：{error}"))?;
    }
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "kind": "ai-novel-project",
        "projectId": random_uuid_v4(),
        "createdAt": crate::commands::iso8601_utc_from_millis(crate::commands::epoch_millis_now()),
    });
    let body = format!(
        "{}\n",
        serde_json::to_string_pretty(&manifest).unwrap_or_default()
    );

    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&manifest_file)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(body.as_bytes())
                .map_err(|error| format!("写入项目清单失败：{error}"))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err("项目清单已存在".to_string());
        }
        Err(error) => return Err(format!("创建项目清单失败：{error}")),
    }

    Ok(TrustedProject {
        project_id: manifest
            .get("projectId")
            .and_then(|item| item.as_str())
            .unwrap_or_default()
            .to_string(),
        root_path: root_path.to_string(),
    })
}

/// 旧版项目可信指纹探测（只读打开，不建表、不迁移）
fn has_trusted_legacy_sqlite_fingerprint(root_path: &str) -> bool {
    let database_path = crate::db::project_database_path(Path::new(root_path));
    if !database_path.is_file() {
        return false;
    }
    let Ok(canonical) = std::fs::canonicalize(&database_path) else {
        return false;
    };
    if !is_contained_path(root_path, &normalize_canonical(canonical)) {
        return false;
    }
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        &database_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return false;
    };

    let table_names = match conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut names = Vec::new();
            for row in rows {
                names.push(row?);
            }
            Ok(names)
        }) {
        Ok(names) => names,
        Err(_) => return false,
    };
    if !LEGACY_REQUIRED_TABLES
        .iter()
        .all(|required| table_names.iter().any(|name| name == required))
    {
        return false;
    }

    let columns = match conn
        .prepare("PRAGMA table_info(project_core)")
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
            let mut names = Vec::new();
            for row in rows {
                names.push(row?);
            }
            Ok(names)
        }) {
        Ok(names) => names,
        Err(_) => return false,
    };
    LEGACY_PROJECT_CORE_COLUMNS
        .iter()
        .all(|required| columns.iter().any(|name| name == required))
}

/// 探测目录是否为项目根（清单优先，其次旧版指纹）
pub fn probe_existing_project(
    candidate_path: &str,
    home: Option<&str>,
) -> Result<ProjectProbe, String> {
    let root_path = canonical_project_root(candidate_path, home)?;
    let manifest_file = manifest_path(&root_path);

    if manifest_file.is_file() {
        let canonical_manifest = std::fs::canonicalize(&manifest_file)
            .map(normalize_canonical)
            .map_err(|error| format!("项目清单无法解析：{error}"))?;
        if !is_contained_path(&root_path, &canonical_manifest) {
            return Err("项目清单越界，已拒绝打开".to_string());
        }
        let raw = std::fs::read_to_string(&manifest_file)
            .map_err(|_| "项目清单无法读取，已拒绝打开".to_string())?;
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).map_err(|_| "项目清单无法读取，已拒绝打开".to_string())?;
        if !is_project_manifest(&parsed) {
            return Err("项目清单无效，已拒绝打开".to_string());
        }
        return Ok(ProjectProbe::Manifest(TrustedProject {
            project_id: parsed
                .get("projectId")
                .and_then(|item| item.as_str())
                .unwrap_or_default()
                .to_string(),
            root_path,
        }));
    }

    if has_trusted_legacy_sqlite_fingerprint(&root_path) {
        return Ok(ProjectProbe::Legacy { root_path });
    }

    Err(PROJECT_ROOT_REQUIRED_MESSAGE.to_string())
}

/// 收养旧版项目（写清单）；已是清单项目则原样返回
pub fn adopt_legacy_project(
    project: ProjectProbe,
    home: Option<&str>,
) -> Result<TrustedProject, String> {
    match project {
        ProjectProbe::Manifest(trusted) => Ok(trusted),
        ProjectProbe::Legacy { root_path } => {
            let current = probe_existing_project(&root_path, home)?;
            if let ProjectProbe::Manifest(trusted) = current {
                return Ok(trusted);
            }
            match write_manifest(&root_path) {
                Ok(trusted) => Ok(trusted),
                Err(error) if error == "项目清单已存在" => {
                    match probe_existing_project(&root_path, home)? {
                        ProjectProbe::Manifest(trusted) => Ok(trusted),
                        ProjectProbe::Legacy { .. } => {
                            Err("旧版项目清单创建冲突，已拒绝打开".to_string())
                        }
                    }
                }
                Err(error) => Err(error),
            }
        }
    }
}

/// 创建新项目目录并写入清单
pub fn create_project(
    parent_path: &str,
    project_name: &str,
    home: Option<&str>,
) -> Result<TrustedProject, String> {
    let canonical_parent = canonical_existing_directory(parent_path)?;
    let directory_name = sanitize_project_directory_name(project_name);
    let requested_root = Path::new(&canonical_parent).join(&directory_name);

    // 新目录必须严格位于所选父目录内
    let requested_root_text = normalize_canonical(requested_root.clone());
    if requested_root_text == canonical_parent
        || !is_contained_path(&canonical_parent, &requested_root_text)
    {
        return Err("新项目目录必须位于所选父目录内".to_string());
    }
    if requested_root.exists() {
        return Err("项目目录已存在，已拒绝覆盖".to_string());
    }

    std::fs::create_dir_all(&requested_root)
        .map_err(|error| format!("创建项目目录失败：{error}"))?;
    let root_path = canonical_project_root(&normalize_canonical(requested_root), home)?;
    write_manifest(&root_path)
}

/// 删除授权：会话有效 + 目标即当前项目根 + 清单身份一致
pub fn authorize_deletion(
    lease: &ProjectSessionLease,
    candidate_path: &str,
    home: Option<&str>,
) -> Result<String, String> {
    let root_path = canonical_project_root(candidate_path, home)?;
    if project_path_key(&root_path) != project_path_key(&lease.root_path) {
        return Err("删除目标不是当前项目根目录，已拒绝删除".to_string());
    }
    match probe_existing_project(&root_path, home)? {
        ProjectProbe::Manifest(trusted) if trusted.project_id == lease.project_id => Ok(root_path),
        _ => Err("删除目标的项目身份不匹配，已拒绝删除".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "anw-access-{name}-{}-{}",
            std::process::id(),
            random_uuid_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn random_uuid_v4_shape_and_uniqueness_test() {
        let first = random_uuid_v4();
        let second = random_uuid_v4();
        assert!(is_uuid_v4(&first), "生成的 UUID 必须满足 v4 形式：{first}");
        assert!(is_uuid_v4(&second));
        assert_ne!(first, second, "连续生成不应重复");
        assert!(!is_uuid_v4("not-a-uuid"));
        assert!(
            !is_uuid_v4("00000000-0000-3000-8000-000000000000"),
            "version 必须为 4"
        );
    }

    #[test]
    fn sanitize_project_directory_name_matches_baseline_test() {
        assert_eq!(sanitize_project_directory_name("  我的小说  "), "我的小说");
        assert_eq!(sanitize_project_directory_name("a/b\\c"), "c");
        assert_eq!(sanitize_project_directory_name("bad:name?*"), "bad_name__");
        assert_eq!(sanitize_project_directory_name("   "), "未命名项目");
        // 基线 `"/".split(/[\\/]/).pop() || trimmed` → 回落 trimmed="/"，再替换为 "_"
        assert_eq!(sanitize_project_directory_name("/"), "_");
    }

    #[test]
    fn is_contained_path_rejects_sibling_and_accepts_child_test() {
        let root = "C:\\projects\\novel";
        assert!(is_contained_path(root, "C:\\projects\\novel"));
        assert!(is_contained_path(
            root,
            "C:\\projects\\novel\\drafts\\a.txt"
        ));
        assert!(!is_contained_path(root, "C:\\projects\\novel-2\\a.txt"));
        assert!(!is_contained_path(root, "C:\\projects"));
        assert!(
            is_contained_path(root, "c:/PROJECTS/NOVEL/子目录"),
            "大小写与分隔符不敏感"
        );
    }

    #[test]
    fn is_project_manifest_validation_test() {
        let valid = serde_json::json!({
            "schemaVersion": 1,
            "kind": "ai-novel-project",
            "projectId": "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
            "createdAt": "2026-10-06T12:00:00.000Z",
        });
        assert!(is_project_manifest(&valid));

        let mut bad_version = valid.clone();
        bad_version["projectId"] = serde_json::json!("3f2504e0-4f89-31d3-9a0c-0305e82c3301");
        assert!(!is_project_manifest(&bad_version), "非 v4 的 id 应被拒绝");

        let mut bad_kind = valid.clone();
        bad_kind["kind"] = serde_json::json!("other");
        assert!(!is_project_manifest(&bad_kind));

        let mut bad_created = valid.clone();
        bad_created["createdAt"] = serde_json::json!("昨天");
        assert!(!is_project_manifest(&bad_created));
    }

    #[test]
    fn create_and_probe_project_roundtrip_test() {
        let parent = temp_root("create");
        let project = create_project(&parent.to_string_lossy(), "我的/小说:01", None).unwrap();

        assert!(
            manifest_path(&project.root_path).is_file(),
            "清单应写入 .lore/project.json"
        );
        assert!(is_uuid_v4(&project.project_id));

        let probed = probe_existing_project(&project.root_path, None).unwrap();
        match probed {
            ProjectProbe::Manifest(trusted) => assert_eq!(trusted.project_id, project.project_id),
            other => panic!("应探测为清单项目：{other:?}"),
        }

        // 同名目录已存在 → 拒绝覆盖
        let duplicate = create_project(&parent.to_string_lossy(), "我的/小说:01", None);
        assert!(duplicate.unwrap_err().contains("已存在"));

        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn probe_rejects_plain_directory_test() {
        let parent = temp_root("plain");
        let error = probe_existing_project(&parent.to_string_lossy(), None).unwrap_err();
        assert!(error.contains("不是项目根目录"), "错误文案不符：{error}");
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn probe_rejects_disk_root_and_home_test() {
        // Windows 下以驱动器根为候选
        let system_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
        let error = probe_existing_project(&format!("{system_drive}\\"), None).unwrap_err();
        assert!(
            error.contains("磁盘根目录") || error.contains("不是项目根目录"),
            "错误文案不符：{error}"
        );
    }

    #[test]
    fn legacy_fingerprint_is_detected_and_adopted_test() {
        let parent = temp_root("legacy");
        let root = parent.join("旧项目");
        std::fs::create_dir_all(root.join(".lore")).unwrap();
        {
            // 构造旧版指纹：5 张必需表 + project_core 必需列，且无清单
            let conn = rusqlite::Connection::open(crate::db::project_database_path(&root)).unwrap();
            conn.execute_batch(
                "CREATE TABLE project_core (id TEXT, project_name TEXT, genre TEXT, total_chapters INTEGER, character_states TEXT);
                 CREATE TABLE blueprints (chapter_number INTEGER);
                 CREATE TABLE characters (name TEXT);
                 CREATE TABLE contents (id INTEGER);
                 CREATE TABLE drafts (id INTEGER);",
            )
            .unwrap();
        }

        let root_text = normalize_canonical(root.clone());
        let probe = probe_existing_project(&root_text, None).unwrap();
        assert!(
            matches!(probe, ProjectProbe::Legacy { .. }),
            "应识别为旧版项目"
        );

        let adopted = adopt_legacy_project(probe, None).unwrap();
        assert!(is_uuid_v4(&adopted.project_id), "收养后应生成清单身份");
        assert!(manifest_path(&adopted.root_path).is_file());

        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn authorize_deletion_requires_matching_identity_test() {
        let parent = temp_root("delete");
        let project = create_project(&parent.to_string_lossy(), "待删项目", None).unwrap();
        let lease = ProjectSessionLease {
            project_id: project.project_id.clone(),
            root_path: project.root_path.clone(),
            lease_id: random_uuid_v4(),
        };
        assert_eq!(
            authorize_deletion(&lease, &project.root_path, None).unwrap(),
            project.root_path
        );

        let stranger = ProjectSessionLease {
            project_id: random_uuid_v4(),
            root_path: project.root_path.clone(),
            lease_id: random_uuid_v4(),
        };
        assert!(authorize_deletion(&stranger, &project.root_path, None).is_err());

        let _ = std::fs::remove_dir_all(&parent);
    }
}
