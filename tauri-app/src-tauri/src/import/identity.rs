//! 导入来源身份解析（批次 G2b）—— 平移自
//! `electron/repositories/import-source-identity-repository.ts:71-160` 的 `resolveEncodedSources`。
//!
//! 职责：把 G1 解析层产出的**别名摘要**（`location_alias_digest` / `file_alias_digest`）
//! 兑换为**稳定的不透明 `sourceId`**，并派生出 `import_runs` 用的指纹：
//!
//! | 基线 | 本实现 |
//! |---|---|
//! | 以 `locationAliasDigest` 为主查 `import_source_aliases` | 同（原地替换文件仍保留来源身份） |
//! | 未命中再按 `fileAliasDigest` 查（把「改名」链到同一 id） | 同 |
//! | 未命中 → `randomUUID()` | [`random_uuid_v4`] |
//! | `INSERT … ON CONFLICT(alias_digest) DO UPDATE` 两条别名 | 同 |
//! | `sourceFingerprint = HMAC(secret, JSON({version:1,purpose,sourceIds:sorted}))` | **D1′：无密钥 `sha256`（同口径、同键序）** |
//! | `sourceFingerprints[i] = fingerprint([sourceIds[i]])` | 同 |
//! | `legacySourceFingerprints` / `legacyCollectionFingerprint` | **D3′：恒不下发**（两栈项目目录不互通，无遗留身份可桥） |
//!
//! 说明：`sourceId` 是本地新生成的 UUID，指纹只在本项目库内自洽，因此 D1′ 不损失任何能力。

use rusqlite::{Connection, OptionalExtension};

use crate::import::limits::MAX_IMPORT_SOURCE_FILES;
use crate::import::parsing::sha256_hex;
use crate::import::ImportPurpose;
use crate::project_access::random_uuid_v4;

/// 单个来源的别名摘要（G1 的 `location_alias_digest` / `file_alias_digest`）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedImportSourceIdentity {
    pub location_alias_digest: String,
    pub file_alias_digest: Option<String>,
}

/// [`resolve_encoded_sources`] 的产出（对齐基线返回值的非 legacy 部分）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImportSourceIdentity {
    /// 与入参一一对应的不透明来源 id
    pub source_ids: Vec<String>,
    /// 整批来源的集合指纹（`import_runs.source_fingerprint`）
    pub source_fingerprint: String,
    /// 逐来源指纹（`import_run_sources.source_fingerprint`）
    pub source_fingerprints: Vec<String>,
}

/// sha256 小写 hex（64 位）
fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// 对齐基线 `fingerprint(ids)`：**手工拼串**以锁定 JS `JSON.stringify` 的键序
/// （version → purpose → sourceIds）；用 `serde_json` + 结构体会按键名排序，口径就变了。
///
/// 排序用 `sort()`（字节序）：id 是 UUID v4（`[0-9a-f-]`），
/// 字节序与基线 `localeCompare(…, 'en-US')` 一致（`-` < 数字 < 小写字母）。
fn fingerprint(purpose: ImportPurpose, source_ids: &[String]) -> String {
    let mut sorted = source_ids.to_vec();
    sorted.sort();
    let items = sorted
        .iter()
        .map(|id| format!("\"{id}\""))
        .collect::<Vec<_>>()
        .join(",");
    sha256_hex(&format!(
        "{{\"version\":1,\"purpose\":\"{}\",\"sourceIds\":[{}]}}",
        purpose.as_str(),
        items
    ))
}

fn upsert_alias(
    conn: &Connection,
    alias_digest: &str,
    alias_kind: &str,
    source_id: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO import_source_aliases (alias_digest, alias_kind, source_id)
         VALUES (?, ?, ?)
         ON CONFLICT(alias_digest) DO UPDATE SET
           alias_kind = excluded.alias_kind,
           source_id = excluded.source_id",
        rusqlite::params![alias_digest, alias_kind, source_id],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn find_source_id(conn: &Connection, alias_digest: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT source_id FROM import_source_aliases WHERE alias_digest = ?",
        rusqlite::params![alias_digest],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// 平移自 `resolveEncodedSources`（D1′ / D3′ 见模块文档）。
pub fn resolve_encoded_sources(
    conn: &Connection,
    sources: &[EncodedImportSourceIdentity],
    purpose: ImportPurpose,
) -> Result<ResolvedImportSourceIdentity, String> {
    if sources.is_empty() || sources.len() > MAX_IMPORT_SOURCE_FILES {
        return Err("导入来源身份无效".to_string());
    }
    for source in sources {
        if !is_sha256(&source.location_alias_digest)
            || source
                .file_alias_digest
                .as_deref()
                .is_some_and(|value| !is_sha256(value))
        {
            return Err("导入来源别名无效".to_string());
        }
    }

    // 与基线 `db.transaction(() => …)` 对齐：别名表写入与 id 解析必须原子
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let mut source_ids = Vec::with_capacity(sources.len());
    for source in sources {
        // location 为主：原地替换文件仍保留来源身份；file 别名把后来的改名链到同一 id
        let location_hit = find_source_id(&tx, &source.location_alias_digest)?;
        let file_hit = match source.file_alias_digest.as_deref() {
            Some(digest) => find_source_id(&tx, digest)?,
            None => None,
        };
        let source_id = location_hit.or(file_hit).unwrap_or_else(random_uuid_v4);
        upsert_alias(&tx, &source.location_alias_digest, "location", &source_id)?;
        if let Some(digest) = source.file_alias_digest.as_deref() {
            upsert_alias(&tx, digest, "file", &source_id)?;
        }
        source_ids.push(source_id);
    }
    tx.commit().map_err(|error| error.to_string())?;

    let source_fingerprint = fingerprint(purpose, &source_ids);
    let source_fingerprints = source_ids
        .iter()
        .map(|id| fingerprint(purpose, std::slice::from_ref(id)))
        .collect();
    Ok(ResolvedImportSourceIdentity {
        source_ids,
        source_fingerprint,
        source_fingerprints,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn memory_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库应可打开");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("应启用外键");
        schema::create_tables(&conn).expect("建表应成功");
        conn
    }

    fn digest(prefix: &str) -> String {
        format!("{prefix}{}", "0".repeat(64 - prefix.len()))
    }

    fn source(location: &str, file: Option<&str>) -> EncodedImportSourceIdentity {
        EncodedImportSourceIdentity {
            location_alias_digest: digest(location),
            file_alias_digest: file.map(digest),
        }
    }

    fn alias_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM import_source_aliases", [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn first_resolve_creates_id_and_both_aliases_test() {
        let conn = memory_conn();
        let resolved = resolve_encoded_sources(
            &conn,
            &[source("aaaa", Some("bbbb"))],
            ImportPurpose::Reference,
        )
        .unwrap();
        assert_eq!(resolved.source_ids.len(), 1);
        assert_eq!(alias_count(&conn), 2, "location + file 两条别名");
        // 第二次同样输入 → 复用同一 id（幂等）
        let again = resolve_encoded_sources(
            &conn,
            &[source("aaaa", Some("bbbb"))],
            ImportPurpose::Reference,
        )
        .unwrap();
        assert_eq!(again.source_ids, resolved.source_ids);
        assert_eq!(alias_count(&conn), 2);
    }

    #[test]
    fn resolves_identity_across_rename_and_in_place_replace_test() {
        let conn = memory_conn();
        let first = resolve_encoded_sources(
            &conn,
            &[source("aaaa", Some("bbbb"))],
            ImportPurpose::Reference,
        )
        .unwrap();

        // 改名：location 变、file 不变 → 命中 file 别名，保留同一来源身份
        let renamed = resolve_encoded_sources(
            &conn,
            &[source("cccc", Some("bbbb"))],
            ImportPurpose::Reference,
        )
        .unwrap();
        assert_eq!(renamed.source_ids, first.source_ids, "改名应保留 sourceId");

        // 原地替换文件：location 不变、file 变 → 命中 location 别名
        let replaced = resolve_encoded_sources(
            &conn,
            &[source("aaaa", Some("dddd"))],
            ImportPurpose::Reference,
        )
        .unwrap();
        assert_eq!(
            replaced.source_ids, first.source_ids,
            "原地替换应保留 sourceId"
        );
    }

    #[test]
    fn rejects_invalid_aliases_and_bounds_test() {
        let conn = memory_conn();
        assert_eq!(
            resolve_encoded_sources(&conn, &[], ImportPurpose::Reference).unwrap_err(),
            "导入来源身份无效"
        );
        let bad = EncodedImportSourceIdentity {
            location_alias_digest: "not-a-digest".to_string(),
            file_alias_digest: None,
        };
        assert_eq!(
            resolve_encoded_sources(&conn, &[bad], ImportPurpose::Reference).unwrap_err(),
            "导入来源别名无效"
        );
        let bad_file = EncodedImportSourceIdentity {
            location_alias_digest: digest("aaaa"),
            file_alias_digest: Some("ZZ".to_string()),
        };
        assert_eq!(
            resolve_encoded_sources(&conn, &[bad_file], ImportPurpose::Reference).unwrap_err(),
            "导入来源别名无效"
        );
        assert_eq!(alias_count(&conn), 0, "校验失败不得写别名表");
    }

    #[test]
    fn fingerprint_matches_canonical_json_and_is_purpose_scoped_test() {
        let conn = memory_conn();
        let resolved = resolve_encoded_sources(
            &conn,
            &[source("aaaa", None), source("bbbb", None)],
            ImportPurpose::Reference,
        )
        .unwrap();
        // 黄金口径：sha256(JSON.stringify({version:1,purpose,sourceIds:sorted}))
        let mut sorted = resolved.source_ids.clone();
        sorted.sort();
        let expected = sha256_hex(&format!(
            "{{\"version\":1,\"purpose\":\"reference\",\"sourceIds\":[\"{}\",\"{}\"]}}",
            sorted[0], sorted[1]
        ));
        assert_eq!(resolved.source_fingerprint, expected);
        assert_eq!(resolved.source_fingerprint.len(), 64);

        // 逐来源指纹 = 单元素集合指纹
        let single = fingerprint(
            ImportPurpose::Reference,
            std::slice::from_ref(&resolved.source_ids[0]),
        );
        assert_eq!(resolved.source_fingerprints[0], single);

        // 同一批来源 + 不同 purpose → 指纹不同
        let author = resolve_encoded_sources(
            &conn,
            &[source("aaaa", None), source("bbbb", None)],
            ImportPurpose::AuthorManuscript,
        )
        .unwrap();
        assert_eq!(author.source_ids, resolved.source_ids, "身份与用途无关");
        assert_ne!(
            author.source_fingerprint, resolved.source_fingerprint,
            "指纹须含 purpose"
        );
        assert_ne!(
            author.source_fingerprints[0],
            resolved.source_fingerprints[0]
        );
    }

    #[test]
    fn fingerprint_is_order_insensitive_test() {
        let conn = memory_conn();
        let forward = resolve_encoded_sources(
            &conn,
            &[source("aaaa", None), source("bbbb", None)],
            ImportPurpose::Reference,
        )
        .unwrap();
        // 反序输入 → sourceIds 与整批指纹一致（逐来源指纹也按各自 id 对齐）
        let reversed = resolve_encoded_sources(
            &conn,
            &[source("bbbb", None), source("aaaa", None)],
            ImportPurpose::Reference,
        )
        .unwrap();
        assert_eq!(reversed.source_ids[0], forward.source_ids[1]);
        assert_eq!(reversed.source_fingerprint, forward.source_fingerprint);
    }
}
