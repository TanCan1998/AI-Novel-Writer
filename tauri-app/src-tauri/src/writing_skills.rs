//! Writing Skill 检查与 GitHub 地址解析 —— 平移自 `src/shared/writing-skills.ts`。
//!
//! 该模块是 `src/shared/writing-skills.ts` 的 **Rust 单源**（纯函数、不依赖 Tauri/DB），
//! 供 `commands/app_data.rs` 的 `skills:inspect-github` / `skills:install-github` 使用。
//!
//! 对齐要点：
//! - frontmatter 解析与字段优先级（`display_name` → `display-name`；`description` 缺省文案）；
//! - 语言归一（en/english → `en-US`，zh/chinese → `zh-CN`，其余 `bilingual`）；
//! - 建议阶段（显式 `stage` 优先，否则按关键词启发式）；
//! - 六类不兼容原因（相对引用 / 脚本 / 钩子 / 子代理 / 工具 / 体积超限）；
//! - GitHub 地址解析（`github.com` 的 repo / tree / blob 与 `raw.githubusercontent.com`），
//!   路径段 percent-decode 后必须是安全段，且最终必须落到 `SKILL.md`。

use serde::{Deserialize, Serialize};

/// Writing Skill 可建议的阶段（对齐 `WRITING_SKILL_STAGES`）
pub const WRITING_SKILL_STAGES: [&str; 4] = ["planning", "drafting", "review", "refinement"];
/// SKILL.md 体积上限（对齐 `MAX_SKILL_BYTES = 64 KiB`）
pub const MAX_SKILL_BYTES: usize = 64 * 1024;
/// URL 长度上限（对齐控制器里的 `sourceUrl.length > 2048` 校验）
pub const MAX_SOURCE_URL_CHARS: usize = 2_048;

/// 对齐契约 `WritingSkillMetadata`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WritingSkillMetadata {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
}

/// 对齐契约 `WritingSkillInspection`（本地检查结果，含正文）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WritingSkillInspection {
    pub metadata: WritingSkillMetadata,
    pub content: String,
    pub compatible: bool,
    pub reasons: Vec<String>,
    pub suggested_stage: String,
    pub utf8_bytes: usize,
}

/// 对齐契约 `RemoteWritingSkillInspection`（远程检查结果：不含正文，附来源指纹）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteWritingSkillInspection {
    pub metadata: WritingSkillMetadata,
    pub compatible: bool,
    pub reasons: Vec<String>,
    pub suggested_stage: String,
    pub utf8_bytes: usize,
    pub source_url: String,
    pub resolved_url: String,
    pub content_sha256: String,
}

/// 对齐契约 `GitHubWritingSkillLocation`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubWritingSkillLocation {
    pub owner: String,
    pub repo: String,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    pub path: String,
    pub source_url: String,
}

/// frontmatter 字段行（对齐 `/^\s*([^:#][^:]*):\s*(.*?)\s*$/`，键统一小写）
fn parse_frontmatter(raw: &str) -> (std::collections::HashMap<String, String>, String) {
    let block = regex::Regex::new(r"(?s)^---\s*\r?\n(.*?)\r?\n---\s*(?:\r?\n|$)").unwrap();
    let Some(captures) = block.captures(raw) else {
        return (std::collections::HashMap::new(), raw.trim().to_string());
    };
    let field_line = regex::Regex::new(r"^\s*([^:#][^:]*):\s*(.*?)\s*$").unwrap();
    let mut fields = std::collections::HashMap::new();
    for line in captures[1].split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(field) = field_line.captures(line) {
            fields.insert(field[1].trim().to_lowercase(), unquote(&field[2]));
        }
    }
    let matched = captures.get(0).map(|item| item.end()).unwrap_or(0);
    (fields, raw[matched..].trim().to_string())
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    if trimmed.len() >= 2 {
        let first = bytes[0];
        let last = bytes[trimmed.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return trimmed[1..trimmed.len() - 1].to_string();
        }
    }
    trimmed.to_string()
}

fn normalized_language(value: Option<&String>) -> String {
    let normalized = value.map(|item| item.to_lowercase()).unwrap_or_default();
    match normalized.as_str() {
        "en-us" | "en" | "english" => "en-US".to_string(),
        "zh-cn" | "zh" | "chinese" => "zh-CN".to_string(),
        _ => "bilingual".to_string(),
    }
}

fn normalized_stage(value: Option<&String>) -> Option<String> {
    let normalized = value?.to_lowercase();
    WRITING_SKILL_STAGES
        .iter()
        .find(|stage| **stage == normalized)
        .map(|stage| (*stage).to_string())
}

fn suggested_stage(fields: &std::collections::HashMap<String, String>, content: &str) -> String {
    if let Some(explicit) = normalized_stage(fields.get("stage")) {
        return explicit;
    }
    let head: String = content.chars().take(1000).collect();
    let searchable = format!(
        "{} {} {}",
        fields.get("name").map(String::as_str).unwrap_or_default(),
        fields.get("description").map(String::as_str).unwrap_or_default(),
        head
    )
    .to_lowercase();
    let matches = |pattern: &str| regex::Regex::new(pattern).unwrap().is_match(&searchable);
    if matches("review|critique|审稿|审阅|检查") {
        return "review".to_string();
    }
    if matches("refin|polish|prose|润色|修稿|改写") {
        return "refinement".to_string();
    }
    if matches("plan|outline|architect|规划|大纲|设定") {
        return "planning".to_string();
    }
    "drafting".to_string()
}

fn matches_any(content: &str, patterns: &[&str]) -> bool {
    patterns
        .iter()
        .any(|pattern| regex::Regex::new(pattern).unwrap().is_match(content))
}

/// 对齐 `inspectWritingSkillMarkdown`
pub fn inspect_writing_skill_markdown(raw: &str) -> Result<WritingSkillInspection, String> {
    let (fields, content) = parse_frontmatter(raw);
    let name = fields
        .get("name")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unnamed-writing-skill".to_string());
    let mut reasons: Vec<String> = Vec::new();
    let byte_length = raw.len();
    let body = content.to_lowercase();
    let declared_capabilities = fields
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    if byte_length > MAX_SKILL_BYTES {
        reasons.push("content-too-large".to_string());
    }
    if matches_any(
        &content,
        &[
            r"(?i)\]\s*\(\s*(?:\.\.?[/\\]|(?:references?|assets?)[/\\])",
            r#"(?im)(?:^|[\s`'"(])(?:references?|assets?)[/\\][^\s`'")]+"#,
            r"(?i)\$\{skill_dir\}",
        ],
    ) {
        reasons.push("relative-reference".to_string());
    }
    if matches_any(
        &content,
        &[
            r#"(?im)(?:^|[\s`'"(])scripts?[/\\][^\s`'")]+"#,
            r"(?i)\b(?:run|execute)\s+(?:the\s+)?script\b",
            r"运行.{0,12}脚本",
        ],
    ) {
        reasons.push("script-dependency".to_string());
    }
    if matches_any(
        &content,
        &[
            r#"(?im)(?:^|[\s`'"(])hooks?[/\\][^\s`'")]+"#,
            r"(?i)\b(?:install|run|execute)\s+(?:the\s+)?hook\b",
            r"安装.{0,12}钩子",
        ],
    ) {
        reasons.push("hook-dependency".to_string());
    }
    if matches_any(&body, &[r"\bsub-?agents?\b|\bdelegate\b.{0,30}\bagents?\b|子代理|子智能体"]) {
        reasons.push("subagent-dependency".to_string());
    }
    if matches_any(
        &declared_capabilities,
        &[r"\ballowed[-_ ]?tools?\b|\btools?\b|\bmcp\b|\bhooks?\b|\bscripts?\b|\bsub-?agents?\b"],
    ) || matches_any(
        &content,
        &[
            r"(?i)\b(?:use|call|invoke)\s+(?:the\s+)?[a-z0-9_-]+\s+tools?\b",
            r"(?:使用|调用).{0,24}工具",
        ],
    ) {
        reasons.push("tool-dependency".to_string());
    }

    let stage = suggested_stage(&fields, &content);
    Ok(WritingSkillInspection {
        metadata: WritingSkillMetadata {
            name: name.clone(),
            display_name: fields
                .get("display_name")
                .or_else(|| fields.get("display-name"))
                .cloned(),
            description: fields
                .get("description")
                .cloned()
                .unwrap_or_else(|| format!("Writing skill: {name}")),
            version: fields.get("version").cloned(),
            language: normalized_language(fields.get("language")),
            stage: normalized_stage(fields.get("stage")),
        },
        compatible: reasons.is_empty(),
        reasons,
        suggested_stage: stage,
        utf8_bytes: content.len(),
        content,
    })
}

/// `decodeURIComponent` 的最小等价实现（%XX → 字节 → UTF-8）
fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("Invalid percent escape in GitHub URL".to_string());
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).map_err(|_| "Invalid percent escape in GitHub URL".to_string())?;
            let decoded = u8::from_str_radix(hex, 16).map_err(|_| "Invalid percent escape in GitHub URL".to_string())?;
            out.push(decoded);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "Invalid percent escape in GitHub URL".to_string())
}

fn safe_part(value: &str, label: &str) -> Result<String, String> {
    let decoded = percent_decode(value)?;
    let safe = regex::Regex::new(r"^[A-Za-z0-9._-]+$").unwrap();
    if !safe.is_match(&decoded) || decoded == "." || decoded == ".." {
        return Err(format!("Invalid GitHub {label}"));
    }
    Ok(decoded)
}

fn safe_path(parts: &[String]) -> Result<String, String> {
    let decoded = parts
        .iter()
        .enumerate()
        .map(|(index, part)| safe_part(part, &format!("path segment {}", index + 1)))
        .collect::<Result<Vec<_>, _>>()?
        .join("/");
    if decoded.is_empty() || !decoded.to_lowercase().ends_with("skill.md") {
        return Err("The GitHub source must resolve to a SKILL.md file".to_string());
    }
    Ok(decoded)
}

/// 对齐 `parseGitHubWritingSkillUrl`
pub fn parse_github_writing_skill_url(value: &str) -> Result<GitHubWritingSkillLocation, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Invalid GitHub URL".to_string())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err("Only public HTTPS GitHub URLs are supported".to_string());
    }
    let parts: Vec<String> = url
        .path()
        .split('/')
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect();
    let hostname = url.host_str().unwrap_or_default().to_string();

    if hostname == "raw.githubusercontent.com" {
        if parts.len() < 4 {
            return Err("Incomplete raw GitHub URL".to_string());
        }
        let repo = parts[1].clone();
        let repo = regex::Regex::new(r"(?i)\.git$").unwrap().replace(&repo, "").to_string();
        return Ok(GitHubWritingSkillLocation {
            owner: safe_part(&parts[0], "owner")?,
            repo: safe_part(&repo, "repository")?,
            reference: Some(safe_part(&parts[2], "ref")?),
            path: safe_path(&parts[3..])?,
            source_url: url.to_string(),
        });
    }
    if hostname != "github.com" || parts.len() < 2 {
        return Err("Only github.com and raw.githubusercontent.com are supported".to_string());
    }

    let repo = parts[1].clone();
    let repo = regex::Regex::new(r"(?i)\.git$").unwrap().replace(&repo, "").to_string();
    let owner = safe_part(&parts[0], "owner")?;
    let repo = safe_part(&repo, "repository")?;
    if parts.len() == 2 {
        return Ok(GitHubWritingSkillLocation {
            owner,
            repo,
            reference: None,
            path: "SKILL.md".to_string(),
            source_url: url.to_string(),
        });
    }

    let kind = parts[2].as_str();
    if (kind != "tree" && kind != "blob") || parts.len() < 4 {
        return Err("Use a GitHub repository, directory, blob, or raw SKILL.md URL".to_string());
    }
    let reference = safe_part(&parts[3], "ref")?;
    let mut target: Vec<String> = parts[4..].to_vec();
    if kind == "tree" {
        target.push("SKILL.md".to_string());
    }
    Ok(GitHubWritingSkillLocation {
        owner,
        repo,
        reference: Some(reference),
        path: safe_path(&target)?,
        source_url: url.to_string(),
    })
}

/// 对齐控制器 `githubRawUrl`
pub fn github_raw_url(owner: &str, repo: &str, reference: &str, path: &str) -> String {
    let mut segments: Vec<String> = vec![owner.to_string(), repo.to_string(), reference.to_string()];
    segments.extend(path.split('/').map(|segment| segment.to_string()));
    let encoded: Vec<String> = segments
        .iter()
        .map(|segment| {
            // 与 JS `encodeURIComponent` 对齐：仅保留 A-Za-z0-9-_.!~*'()
            segment
                .bytes()
                .map(|byte| {
                    let character = byte as char;
                    if character.is_ascii_alphanumeric()
                        || matches!(character, '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')')
                    {
                        character.to_string()
                    } else {
                        format!("%{byte:02X}")
                    }
                })
                .collect::<String>()
        })
        .collect();
    format!("https://raw.githubusercontent.com/{}", encoded.join("/"))
}

/// 对齐契约 `InstalledWritingSkill`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledWritingSkill {
    pub name: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub language: String,
    pub compatible: bool,
    pub utf8_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\nname: fog-harbor\nversion: 1.2.0\nlanguage: zh\nstage: planning\ndescription: 灯语线索规划\n---\n\n写前先列线索。\n";

    #[test]
    fn inspect_parses_frontmatter_and_metadata_test() {
        let inspected = inspect_writing_skill_markdown(SAMPLE).unwrap();
        assert_eq!(inspected.metadata.name, "fog-harbor");
        assert_eq!(inspected.metadata.version.as_deref(), Some("1.2.0"));
        assert_eq!(inspected.metadata.language, "zh-CN");
        assert_eq!(inspected.metadata.stage.as_deref(), Some("planning"));
        assert_eq!(inspected.metadata.description, "灯语线索规划");
        assert_eq!(inspected.suggested_stage, "planning");
        assert!(inspected.compatible, "{:?}", inspected.reasons);
        assert_eq!(inspected.content, "写前先列线索。");
        assert_eq!(inspected.utf8_bytes, inspected.content.len());
    }

    #[test]
    fn inspect_without_frontmatter_uses_defaults_test() {
        let inspected = inspect_writing_skill_markdown("纯正文，无 frontmatter").unwrap();
        assert_eq!(inspected.metadata.name, "unnamed-writing-skill");
        assert_eq!(inspected.metadata.language, "bilingual");
        assert_eq!(inspected.metadata.stage, None);
        assert_eq!(inspected.metadata.description, "Writing skill: unnamed-writing-skill");
        assert_eq!(inspected.suggested_stage, "drafting");
        assert_eq!(inspected.content, "纯正文，无 frontmatter");
    }

    #[test]
    fn inspect_reports_each_incompatibility_reason_test() {
        let relative = inspect_writing_skill_markdown("见 [参考](references/a.md)").unwrap();
        assert!(relative.reasons.contains(&"relative-reference".to_string()));

        let script = inspect_writing_skill_markdown("先运行 `scripts/build.py` 再继续").unwrap();
        assert!(script.reasons.contains(&"script-dependency".to_string()));

        let hook = inspect_writing_skill_markdown("请安装钩子后再用").unwrap();
        assert!(hook.reasons.contains(&"hook-dependency".to_string()));

        let subagent = inspect_writing_skill_markdown("交给子代理处理").unwrap();
        assert!(subagent.reasons.contains(&"subagent-dependency".to_string()));

        let tool = inspect_writing_skill_markdown("请调用 grep 工具").unwrap();
        assert!(tool.reasons.contains(&"tool-dependency".to_string()));
        assert!(!tool.compatible);
    }

    #[test]
    fn inspect_flags_oversized_content_test() {
        let big = "a".repeat(MAX_SKILL_BYTES + 1);
        let inspected = inspect_writing_skill_markdown(&big).unwrap();
        assert!(inspected.reasons.contains(&"content-too-large".to_string()));
    }

    #[test]
    fn parse_github_url_accepts_supported_forms_test() {
        let repo = parse_github_writing_skill_url("https://github.com/o/r").unwrap();
        assert_eq!((repo.owner.as_str(), repo.repo.as_str(), repo.path.as_str()), ("o", "r", "SKILL.md"));
        assert_eq!(repo.reference, None);

        let tree = parse_github_writing_skill_url("https://github.com/o/r/tree/main/skills/a").unwrap();
        assert_eq!(tree.reference.as_deref(), Some("main"));
        assert_eq!(tree.path, "skills/a/SKILL.md");

        let blob = parse_github_writing_skill_url("https://github.com/o/r/blob/main/skills/a/SKILL.md").unwrap();
        assert_eq!(blob.path, "skills/a/SKILL.md");

        let raw = parse_github_writing_skill_url("https://raw.githubusercontent.com/o/r/v1/SKILL.md").unwrap();
        assert_eq!(raw.reference.as_deref(), Some("v1"));

        // .git 后缀归一
        let dotted = parse_github_writing_skill_url("https://github.com/o/r.git").unwrap();
        assert_eq!(dotted.repo, "r");

        // 百分号编码段
        let encoded = parse_github_writing_skill_url("https://github.com/o/r/tree/main/skills%2Da").unwrap();
        assert_eq!(encoded.path, "skills-a/SKILL.md");
    }

    #[test]
    fn parse_github_url_rejects_unsupported_forms_test() {
        let cases = [
            "http://github.com/o/r",
            "https://user:pw@github.com/o/r",
            "https://github.com:8443/o/r",
            "https://gitlab.com/o/r",
            "https://github.com/o",
            "https://github.com/o/r/tree",
            "https://raw.githubusercontent.com/o/r/v1",
            "https://github.com/o/r/blob/main/notes.md",
            "https://github.com/o/r/tree/main/..%2Fetc",
        ];
        for case in cases {
            assert!(
                parse_github_writing_skill_url(case).is_err(),
                "应拒绝：{case}"
            );
        }
        // `tree` 指向目录时自动追加 SKILL.md（对齐基线的宽容行为）
        let tree_dir = parse_github_writing_skill_url("https://github.com/o/r/tree/main/notes").unwrap();
        assert_eq!(tree_dir.path, "notes/SKILL.md");
        // https 默认端口被 URL 规范化去除（与 JS `new URL` 一致）
        assert!(parse_github_writing_skill_url("https://github.com:443/o/r").is_ok());
    }

    #[test]
    fn github_raw_url_encodes_segments_test() {
        assert_eq!(
            github_raw_url("o", "r", "main", "skills/a/SKILL.md"),
            "https://raw.githubusercontent.com/o/r/main/skills/a/SKILL.md"
        );
        assert_eq!(
            github_raw_url("o", "r", "feat/x", "SKILL.md"),
            "https://raw.githubusercontent.com/o/r/feat%2Fx/SKILL.md"
        );
    }
}
