//! 批次 H：应用数据域 —— `prompt:*`（3）+ `skills:*`（4），平移自
//! `electron/controllers/app-data-controller.ts`。
//!
//! **能力域**：`~/.lorekeeper/prompts` 与 `~/.lorekeeper/skills` 只能由本模块访问；
//! 渲染层不接收任意 app-data 路径，也不借用外部文件授权（对齐基线注释）。
//!
//! 失败信封口径（逐条对齐基线）：
//! - `prompt:save-global` / `prompt:delete-global` → `String(error)` ⇒ **带 `Error: ` 前缀**；
//! - `skills:*` 四个频道 → `error.message` ⇒ **不带前缀**；
//! - `prompt:load-global` 的 `diagnostics[].error` → `error.message` ⇒ 不带前缀；
//! - `skills:list-user` 的根目录校验失败会**直接 reject**（基线无 try/catch）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::app_paths;
use crate::commands::db::simple_mutating_result;
use crate::commands::SimpleResult;
use crate::llm::chat::{build_client_with_timeout, proxy_from_config};
use crate::security::{lexically_contained, lexically_normalize};
use crate::state::AppState;
use crate::writing_skills::{
    github_raw_url, inspect_writing_skill_markdown, parse_github_writing_skill_url,
    RemoteWritingSkillInspection, MAX_SKILL_BYTES, MAX_SOURCE_URL_CHARS,
};

/// 对齐基线 `WRITING_SKILL_NAME`
fn writing_skill_name_pattern() -> regex::Regex {
    regex::Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$").unwrap()
}

/// 对齐基线 `promptKeyPath` 的标识校验
fn prompt_key_pattern() -> regex::Regex {
    regex::Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$").unwrap()
}

fn prompts_directory() -> PathBuf {
    app_paths::lorekeeper_home().join("prompts")
}

fn is_contained(root: &Path, candidate: &Path) -> bool {
    lexically_contained(
        &lexically_normalize(&root.to_string_lossy()),
        &lexically_normalize(&candidate.to_string_lossy()),
    )
}

fn is_prompt_template(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if !object.get("key").map(Value::is_string).unwrap_or(false) {
        return false;
    }
    match object.get("writingLanguage") {
        None => true,
        Some(item) => matches!(item.as_str(), Some("zh-CN") | Some("en-US")),
    }
}

/// 对齐基线 `promptFilename`（`writingLanguage` 为空串视为缺省）
fn prompt_filename(template: &Value) -> Option<String> {
    let key = template.get("key")?.as_str()?;
    let language = template
        .get("writingLanguage")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    Some(match language {
        Some(language) => format!("{key}.{language}"),
        None => key.to_string(),
    })
}

fn prompt_key_from_filename(filename: &str) -> String {
    let stem = filename.strip_suffix(".json").unwrap_or(filename);
    let language = regex::Regex::new(r"\.(?:zh-CN|en-US)$").unwrap();
    language.replace(stem, "").to_string()
}

fn prompt_language_from_filename(filename: &str) -> Option<String> {
    let matched = regex::Regex::new(r"\.(zh-CN|en-US)\.json$").unwrap();
    matched
        .captures(filename)
        .map(|captures| captures[1].to_string())
}

/// 对齐契约 `PromptLoadDiagnostic`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptLoadDiagnostic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writing_language: Option<String>,
    pub path: String,
    pub error: String,
}

/// 对齐契约 `AppPromptLoadReceipt`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPromptLoadReceipt {
    pub templates: Vec<Value>,
    pub diagnostics: Vec<PromptLoadDiagnostic>,
}

/// 对齐契约 `skills:list-user` 的元素
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserWritingSkill {
    pub name: String,
    pub content: String,
    pub base_dir: String,
    pub file_path: String,
}

// ===== 逻辑层（不依赖 Tauri State，便于单测）=====

/// `prompt:load-global` 逻辑
pub fn load_global_prompts_at(directory: &Path) -> AppPromptLoadReceipt {
    if !directory.exists() {
        return AppPromptLoadReceipt {
            templates: Vec::new(),
            diagnostics: Vec::new(),
        };
    }
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            return AppPromptLoadReceipt {
                templates: Vec::new(),
                diagnostics: vec![PromptLoadDiagnostic {
                    key: None,
                    writing_language: None,
                    path: "prompts".to_string(),
                    error: error.to_string(),
                }],
            }
        }
    };
    let canonical_directory =
        std::fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
    let mut templates: Vec<Value> = Vec::new();
    let mut diagnostics: Vec<PromptLoadDiagnostic> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
            || !name.ends_with(".json")
        {
            continue;
        }
        let outcome: Result<Value, String> = (|| {
            let candidate = directory.join(&name);
            let canonical = std::fs::canonicalize(&candidate).map_err(|error| error.to_string())?;
            if !lexically_contained(&canonical_directory, &canonical) {
                return Err("提示词目标超出应用目录".to_string());
            }
            let raw = std::fs::read_to_string(&canonical).map_err(|error| error.to_string())?;
            let parsed: Value =
                serde_json::from_str(&raw).map_err(|_| "提示词内容结构无效".to_string())?;
            if !is_prompt_template(&parsed) {
                return Err("提示词内容结构无效".to_string());
            }
            let filename_key = name.strip_suffix(".json").unwrap_or(&name).to_string();
            if prompt_filename(&parsed).as_deref() != Some(filename_key.as_str()) {
                return Err("提示词标识或语言与文件名不一致".to_string());
            }
            Ok(parsed)
        })();
        match outcome {
            Ok(template) => templates.push(template),
            Err(error) => diagnostics.push(PromptLoadDiagnostic {
                key: Some(prompt_key_from_filename(&name)),
                writing_language: prompt_language_from_filename(&name),
                path: name,
                error,
            }),
        }
    }
    AppPromptLoadReceipt {
        templates,
        diagnostics,
    }
}

/// `prompt:save-global` 逻辑
pub fn save_global_prompt_at(directory: &Path, template: &Value) -> Result<(), String> {
    if !is_prompt_template(template) {
        return Err("提示词内容无效".to_string());
    }
    let key = template
        .get("key")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let language = template
        .get("writingLanguage")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("zh-CN");
    let mut stored = template.clone();
    if let Some(object) = stored.as_object_mut() {
        object.insert(
            "writingLanguage".to_string(),
            Value::String(language.to_string()),
        );
    }
    let target = prompt_key_path_in(directory, key, Some(language))?;
    crate::json_store::write_json_file(&target, &stored)?;
    if language == "zh-CN" {
        let legacy = prompt_key_path_in(directory, key, None)?;
        if legacy.exists() {
            std::fs::remove_file(&legacy).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// `prompt:delete-global` 逻辑
pub fn delete_global_prompt_at(
    directory: &Path,
    key: &str,
    writing_language: &str,
) -> Result<(), String> {
    let mut paths = vec![prompt_key_path_in(directory, key, Some(writing_language))?];
    if writing_language == "zh-CN" {
        paths.push(prompt_key_path_in(directory, key, None)?);
    }
    for path in paths {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// 目录参数化的 `promptKeyPath`（便于单测）
fn prompt_key_path_in(
    directory: &Path,
    key: &str,
    writing_language: Option<&str>,
) -> Result<PathBuf, String> {
    if !prompt_key_pattern().is_match(key) || key == "." || key == ".." {
        return Err("提示词标识无效".to_string());
    }
    let suffix = writing_language
        .map(|language| format!(".{language}"))
        .unwrap_or_default();
    let candidate = directory.join(format!("{key}{suffix}.json"));
    if !is_contained(directory, &candidate) {
        return Err("提示词目标超出应用目录".to_string());
    }
    Ok(candidate)
}

/// 对齐基线 `ensureOwnedSkillsRoot`
fn ensure_owned_skills_root_at(home: &Path) -> Result<PathBuf, String> {
    if home.exists() {
        let info = std::fs::symlink_metadata(home).map_err(|error| error.to_string())?;
        if info.file_type().is_symlink() || !info.is_dir() {
            return Err("应用数据目录不是受信任的本地目录".to_string());
        }
    } else {
        std::fs::create_dir_all(home).map_err(|error| error.to_string())?;
    }
    let canonical_home = std::fs::canonicalize(home).map_err(|error| error.to_string())?;
    let skills_root = home.join("skills");
    if skills_root.exists() {
        let info = std::fs::symlink_metadata(&skills_root).map_err(|error| error.to_string())?;
        if info.file_type().is_symlink() || !info.is_dir() {
            return Err("用户 Skill 目录不是受信任的本地目录".to_string());
        }
    } else {
        std::fs::create_dir(&skills_root).map_err(|error| error.to_string())?;
    }
    let canonical_root = std::fs::canonicalize(&skills_root).map_err(|error| error.to_string())?;
    if !lexically_contained(&canonical_home, &canonical_root) {
        return Err("用户 Skill 目录超出应用目录".to_string());
    }
    Ok(canonical_root)
}

/// 对齐基线 `writingSkillDirectory`
fn writing_skill_directory_at(home: &Path, name: &str) -> Result<PathBuf, String> {
    if !writing_skill_name_pattern().is_match(name) || name == "." || name == ".." {
        return Err("写作 Skill 名称无效".to_string());
    }
    let root = home.join("skills");
    let candidate = root.join(name);
    if !is_contained(&root, &candidate) {
        return Err("写作 Skill 目标超出应用目录".to_string());
    }
    Ok(candidate)
}

/// 对齐基线 `ensureOwnedSkillTarget`
fn ensure_owned_skill_target_at(home: &Path, name: &str) -> Result<(PathBuf, PathBuf), String> {
    let canonical_root = ensure_owned_skills_root_at(home)?;
    let requested = writing_skill_directory_at(home, name)?;
    if requested.exists() {
        let info = std::fs::symlink_metadata(&requested).map_err(|error| error.to_string())?;
        if info.file_type().is_symlink() || !info.is_dir() {
            return Err("拒绝写入链接或非目录 Skill 目标".to_string());
        }
    } else {
        std::fs::create_dir(&requested).map_err(|error| error.to_string())?;
    }
    let directory = std::fs::canonicalize(&requested).map_err(|error| error.to_string())?;
    if !lexically_contained(&canonical_root, &directory) {
        return Err("写作 Skill 目标超出应用目录".to_string());
    }
    let file_path = directory.join("SKILL.md");
    if file_path.exists() {
        let info = std::fs::symlink_metadata(&file_path).map_err(|error| error.to_string())?;
        if info.file_type().is_symlink() || !info.is_file() {
            return Err("拒绝覆盖链接或非文件 SKILL.md".to_string());
        }
        let canonical_file =
            std::fs::canonicalize(&file_path).map_err(|error| error.to_string())?;
        if !lexically_contained(&directory, &canonical_file) {
            return Err("SKILL.md 超出 Skill 目录".to_string());
        }
    }
    Ok((directory, file_path))
}

/// `skills:list-user` 逻辑（返回 `Err` 时对齐基线的 invoke reject）
pub fn list_user_skills_at(home: &Path) -> Result<Vec<UserWritingSkill>, String> {
    let directory = home.join("skills");
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let canonical_root = ensure_owned_skills_root_at(home)?;
    let mut skills = Vec::new();
    let entries = std::fs::read_dir(&canonical_root).map_err(|error| error.to_string())?;
    for entry in entries.flatten() {
        let file_type = match entry.file_type() {
            Ok(kind) => kind,
            Err(_) => continue,
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let attempt: Result<UserWritingSkill, String> = (|| {
            let base_dir =
                std::fs::canonicalize(entry.path()).map_err(|error| error.to_string())?;
            if !lexically_contained(&canonical_root, &base_dir) {
                return Err("写作 Skill 目标超出应用目录".to_string());
            }
            let file_path = std::fs::canonicalize(base_dir.join("SKILL.md"))
                .map_err(|error| error.to_string())?;
            if !lexically_contained(&base_dir, &file_path) {
                return Err("SKILL.md 超出 Skill 目录".to_string());
            }
            if !std::fs::metadata(&file_path)
                .map(|info| info.is_file())
                .unwrap_or(false)
            {
                return Err("SKILL.md 不是文件".to_string());
            }
            Ok(UserWritingSkill {
                name,
                content: std::fs::read_to_string(&file_path).map_err(|error| error.to_string())?,
                base_dir: base_dir.to_string_lossy().to_string(),
                file_path: file_path.to_string_lossy().to_string(),
            })
        })();
        // 单个用户 Skill 无效时不阻断其余 Skill（对齐基线）
        if let Ok(skill) = attempt {
            skills.push(skill);
        }
    }
    Ok(skills)
}

/// `skills:uninstall-user` 逻辑
pub fn uninstall_user_skill_at(home: &Path, name: &str) -> Result<(), String> {
    let directory = writing_skill_directory_at(home, name)?;
    let skills_root = home.join("skills");
    if !skills_root.exists() {
        return Ok(());
    }
    let canonical_root = ensure_owned_skills_root_at(home)?;
    if !directory.exists() {
        return Ok(());
    }
    let info = std::fs::symlink_metadata(&directory).map_err(|error| error.to_string())?;
    if info.file_type().is_symlink() {
        return Err("拒绝删除符号链接 Skill".to_string());
    }
    let canonical_directory =
        std::fs::canonicalize(&directory).map_err(|error| error.to_string())?;
    if !lexically_contained(&canonical_root, &canonical_directory) {
        return Err("写作 Skill 目标超出应用目录".to_string());
    }
    std::fs::remove_dir_all(&canonical_directory).map_err(|error| error.to_string())
}

// ===== GitHub 抓取（对齐基线 `fetchGitHubWritingSkill`）=====

struct FetchedWritingSkill {
    raw: String,
    inspection: RemoteWritingSkillInspection,
}

async fn fetch_github_writing_skill(source_url: &str) -> Result<FetchedWritingSkill, String> {
    let location = parse_github_writing_skill_url(source_url)?;
    let config = crate::json_store::read_json_value_or(
        &app_paths::global_config_path(),
        crate::commands::config::default_global_config(),
    );
    let proxy = proxy_from_config(&config);
    let client = build_client_with_timeout(proxy.as_ref(), Some(Duration::from_secs(10)))?;

    let reference = match location.reference.clone() {
        Some(reference) => reference,
        None => {
            let response = client
                .get(format!(
                    "https://api.github.com/repos/{}/{}",
                    location.owner, location.repo
                ))
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "AI-Novel-Writer")
                .send()
                .await
                .map_err(|error| error.to_string())?;
            if !response.status().is_success() {
                return Err(format!(
                    "GitHub repository lookup failed ({})",
                    response.status().as_u16()
                ));
            }
            let payload: Value = response.json().await.map_err(|error| error.to_string())?;
            let branch = payload
                .get("default_branch")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !writing_skill_name_pattern().is_match(&branch) {
                return Err("GitHub returned an unsupported default branch name".to_string());
            }
            branch
        }
    };

    let resolved_url = github_raw_url(&location.owner, &location.repo, &reference, &location.path);
    let response = client
        .get(&resolved_url)
        .header("Accept", "text/plain")
        .header("User-Agent", "AI-Novel-Writer")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "SKILL.md download failed ({})",
            response.status().as_u16()
        ));
    }
    if let Some(length) = response.content_length() {
        if length as usize > MAX_SKILL_BYTES {
            return Err("SKILL.md is larger than 64 KiB".to_string());
        }
    }
    let bytes = response.bytes().await.map_err(|error| error.to_string())?;
    if bytes.len() > MAX_SKILL_BYTES {
        return Err("SKILL.md is larger than 64 KiB".to_string());
    }
    let raw =
        String::from_utf8(bytes.to_vec()).map_err(|_| "SKILL.md is not valid UTF-8".to_string())?;

    let inspected = inspect_writing_skill_markdown(&raw)?;
    let inspection = RemoteWritingSkillInspection {
        metadata: inspected.metadata,
        compatible: inspected.compatible,
        reasons: inspected.reasons,
        suggested_stage: inspected.suggested_stage,
        utf8_bytes: inspected.utf8_bytes,
        source_url: source_url.to_string(),
        resolved_url,
        content_sha256: crate::repositories::finalized_continuity_repository::sha256_hex(&raw),
    };
    Ok(FetchedWritingSkill { raw, inspection })
}

fn validate_source_url(source_url: &str) -> Result<(), String> {
    if source_url.chars().count() > MAX_SOURCE_URL_CHARS {
        return Err("GitHub 地址无效".to_string());
    }
    Ok(())
}

/// `skills:inspect-github` 的结果信封
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInspectResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inspection: Option<RemoteWritingSkillInspection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `skills:install-github` 的结果信封
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill: Option<crate::writing_skills::InstalledWritingSkill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ===== 命令 =====

/// `prompt:load-global`
#[tauri::command]
pub fn prompt_load_global() -> AppPromptLoadReceipt {
    load_global_prompts_at(&prompts_directory())
}

/// `prompt:save-global`
#[tauri::command]
pub fn prompt_save_global(template: Value) -> SimpleResult {
    simple_mutating_result(save_global_prompt_at(&prompts_directory(), &template))
}

/// `prompt:delete-global`
#[tauri::command]
pub fn prompt_delete_global(key: String, writing_language: String) -> SimpleResult {
    simple_mutating_result(delete_global_prompt_at(
        &prompts_directory(),
        &key,
        &writing_language,
    ))
}

/// `skills:list-user`（根目录不可信时 reject，对齐基线无 try/catch）
#[tauri::command]
pub fn skills_list_user() -> Result<Vec<UserWritingSkill>, String> {
    list_user_skills_at(&app_paths::lorekeeper_home())
}

/// `skills:inspect-github`
#[tauri::command]
pub async fn skills_inspect_github(
    state: State<'_, AppState>,
    source_url: String,
) -> Result<SkillInspectResult, String> {
    let failure = |error: String| SkillInspectResult {
        success: false,
        inspection: None,
        error: Some(error),
    };
    if validate_source_url(&source_url).is_err() {
        return Ok(failure("GitHub 地址无效".to_string()));
    }
    match fetch_github_writing_skill(&source_url).await {
        Ok(fetched) => {
            let mut cache = state
                .writing_skill_inspections
                .lock()
                .map_err(|_| "写作 Skill 检查缓存被污染".to_string())?;
            cache.insert(
                source_url.clone(),
                (
                    fetched.inspection.content_sha256.clone(),
                    fetched.inspection.resolved_url.clone(),
                ),
            );
            Ok(SkillInspectResult {
                success: true,
                inspection: Some(fetched.inspection),
                error: None,
            })
        }
        Err(error) => {
            if let Ok(mut cache) = state.writing_skill_inspections.lock() {
                cache.remove(&source_url);
            }
            Ok(failure(error))
        }
    }
}

/// `skills:install-github`（必须先 inspect：一次性确认缓存 + 重新抓取核对指纹）
#[tauri::command]
pub async fn skills_install_github(
    state: State<'_, AppState>,
    source_url: String,
) -> Result<SkillInstallResult, String> {
    let failure = |error: String| SkillInstallResult {
        success: false,
        skill: None,
        error: Some(error),
    };
    if validate_source_url(&source_url).is_err() {
        return Ok(failure("GitHub 地址无效".to_string()));
    }
    let confirmed = {
        let mut cache = state
            .writing_skill_inspections
            .lock()
            .map_err(|_| "写作 Skill 检查缓存被污染".to_string())?;
        match cache.remove(&source_url) {
            Some(entry) => entry,
            None => {
                return Ok(failure(
                    "请先检查该 GitHub Writing Skill，再确认安装".to_string(),
                ))
            }
        }
    };

    // 确认后重新抓取；渲染层提供的检查结果从不作为安装输入
    let fetched = match fetch_github_writing_skill(&source_url).await {
        Ok(fetched) => fetched,
        Err(error) => return Ok(failure(error)),
    };
    if fetched.inspection.content_sha256 != confirmed.0
        || fetched.inspection.resolved_url != confirmed.1
    {
        return Ok(failure(
            "Writing Skill 在检查后已发生变化，请重新检查".to_string(),
        ));
    }
    if !fetched.inspection.compatible {
        return Ok(failure(format!(
            "该 Skill 不是自包含提示词：{}",
            fetched.inspection.reasons.join(", ")
        )));
    }

    let home = app_paths::lorekeeper_home();
    let name = fetched.inspection.metadata.name.clone();
    let requested = match writing_skill_directory_at(&home, &name) {
        Ok(path) => path,
        Err(error) => return Ok(failure(error)),
    };
    if requested.exists() {
        return Ok(failure(format!("同名 Writing Skill 已安装：{name}")));
    }
    let (_, file_path) = match ensure_owned_skill_target_at(&home, &name) {
        Ok(target) => target,
        Err(error) => return Ok(failure(error)),
    };
    if let Err(error) = std::fs::write(&file_path, fetched.raw.as_bytes()) {
        return Ok(failure(error.to_string()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&file_path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(SkillInstallResult {
        success: true,
        skill: Some(crate::writing_skills::InstalledWritingSkill {
            name,
            source: "user".to_string(),
            version: fetched.inspection.metadata.version.clone(),
            language: fetched.inspection.metadata.language.clone(),
            compatible: true,
            utf8_bytes: fetched.inspection.utf8_bytes,
        }),
        error: None,
    })
}

/// `skills:uninstall-user`
#[tauri::command]
pub fn skills_uninstall_user(name: String) -> SimpleResult {
    // 注意：skills 频道失败**不带** `Error: ` 前缀（对齐基线 `error.message`）
    match uninstall_user_skill_at(&app_paths::lorekeeper_home(), &name) {
        Ok(()) => SimpleResult {
            success: true,
            error: None,
        },
        Err(error) => SimpleResult {
            success: false,
            error: Some(error),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!(
            "lorekeeper-appdata-{name}-{}",
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    #[test]
    fn prompt_roundtrip_load_save_delete_test() {
        let home = temp_home("prompts");
        let prompts = home.join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();

        // 空目录 / 目录不存在
        assert!(load_global_prompts_at(&prompts).templates.is_empty());
        assert!(load_global_prompts_at(&home.join("nope"))
            .diagnostics
            .is_empty());

        // 保存（缺省语言 → zh-CN，并写 `.zh-CN` 后缀文件）
        let template = json!({ "key": "draft-helper", "content": "请先写大纲" });
        save_global_prompt_at(&prompts, &template).unwrap();
        assert!(prompts.join("draft-helper.zh-CN.json").is_file());

        let receipt = load_global_prompts_at(&prompts);
        assert_eq!(receipt.templates.len(), 1);
        assert_eq!(receipt.templates[0]["key"], "draft-helper");
        assert_eq!(receipt.templates[0]["writingLanguage"], "zh-CN");
        assert!(receipt.diagnostics.is_empty());

        // en-US 变体共存
        save_global_prompt_at(
            &prompts,
            &json!({ "key": "draft-helper", "writingLanguage": "en-US" }),
        )
        .unwrap();
        assert_eq!(load_global_prompts_at(&prompts).templates.len(), 2);

        // 保存 zh-CN 会清掉无语言后缀的 legacy 文件
        std::fs::write(prompts.join("draft-helper.json"), "{}").unwrap();
        save_global_prompt_at(&prompts, &template).unwrap();
        assert!(
            !prompts.join("draft-helper.json").exists(),
            "zh-CN 保存应清理 legacy 文件"
        );

        // 删除 zh-CN 同时删 legacy
        std::fs::write(prompts.join("draft-helper.json"), "{}").unwrap();
        delete_global_prompt_at(&prompts, "draft-helper", "zh-CN").unwrap();
        assert!(!prompts.join("draft-helper.zh-CN.json").exists());
        assert!(!prompts.join("draft-helper.json").exists());
        assert_eq!(
            load_global_prompts_at(&prompts).templates.len(),
            1,
            "en-US 变体应保留"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn prompt_load_reports_diagnostics_and_rejects_bad_keys_test() {
        let home = temp_home("prompts-diag");
        let prompts = home.join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();

        std::fs::write(
            prompts.join("ok.zh-CN.json"),
            json!({ "key": "ok", "writingLanguage": "zh-CN" }).to_string(),
        )
        .unwrap();
        // 非法 JSON
        std::fs::write(prompts.join("broken.zh-CN.json"), "{ not json").unwrap();
        // 结构无效
        std::fs::write(
            prompts.join("shape.zh-CN.json"),
            json!({ "key": 7 }).to_string(),
        )
        .unwrap();
        // 标识与文件名不一致
        std::fs::write(
            prompts.join("mismatch.zh-CN.json"),
            json!({ "key": "other" }).to_string(),
        )
        .unwrap();

        let receipt = load_global_prompts_at(&prompts);
        assert_eq!(receipt.templates.len(), 1);
        assert_eq!(receipt.diagnostics.len(), 3);
        assert!(receipt
            .diagnostics
            .iter()
            .all(|item| item.writing_language.as_deref() == Some("zh-CN")));
        assert_eq!(receipt.diagnostics[0].key.as_deref(), Some("broken"));
        assert_eq!(receipt.diagnostics[0].path, "broken.zh-CN.json");

        // 非法 key / 非法模板
        assert_eq!(
            prompt_key_path_in(&prompts, "../escape", None).unwrap_err(),
            "提示词标识无效"
        );
        assert_eq!(
            prompt_key_path_in(&prompts, "..", None).unwrap_err(),
            "提示词标识无效"
        );
        assert_eq!(
            save_global_prompt_at(&prompts, &json!({ "key": "x", "writingLanguage": "ja-JP" }))
                .unwrap_err(),
            "提示词内容无效"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn skills_list_uninstall_and_guards_test() {
        let home = temp_home("skills");
        // 目录不存在 → 空列表
        assert!(list_user_skills_at(&home).unwrap().is_empty());

        let skill_dir = home.join("skills").join("fog-harbor");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: fog-harbor\n---\n正文",
        )
        .unwrap();
        // 无 SKILL.md 的目录被静默跳过
        std::fs::create_dir_all(home.join("skills").join("empty-skill")).unwrap();

        let skills = list_user_skills_at(&home).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "fog-harbor");
        assert!(skills[0].file_path.ends_with("SKILL.md"));

        // 非法名称
        assert_eq!(
            uninstall_user_skill_at(&home, "..").unwrap_err(),
            "写作 Skill 名称无效"
        );
        // 不存在 → 幂等成功
        uninstall_user_skill_at(&home, "not-installed").unwrap();

        uninstall_user_skill_at(&home, "fog-harbor").unwrap();
        assert!(!skill_dir.exists());
        assert_eq!(list_user_skills_at(&home).unwrap().len(), 0);

        // 安装目标：已有同名文件时拒绝覆盖链接/非文件
        let (_, file_path) = ensure_owned_skill_target_at(&home, "new-skill").unwrap();
        assert!(file_path.parent().unwrap().is_dir());
        std::fs::write(&file_path, "x").unwrap();
        ensure_owned_skill_target_at(&home, "new-skill").unwrap();

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn source_url_length_guard_test() {
        assert!(validate_source_url("https://github.com/o/r").is_ok());
        assert_eq!(
            validate_source_url(&"a".repeat(MAX_SOURCE_URL_CHARS + 1)).unwrap_err(),
            "GitHub 地址无效"
        );
    }
}
