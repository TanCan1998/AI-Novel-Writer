//! 批次 G1：作者原稿导入（自 `electron/controllers/import-controller.ts` +
//! `electron/services/import-inspection-store.ts` 平移）。
//!
//! # 交付范围
//!
//! G1 只交付**「导入作者原稿」完整可用链路**：
//! `dialog:select-novel-files` → 检视存储（[`inspection_store`]）→ 章节解析
//! （[`parsing`]）→ `db:import-run-author-preview`（复用批次 E 已迁移的
//! `finalized_draft_import_repository::preview`）。
//!
//! `reference`（参考语料）路径依赖 G2 的导入运行状态机（`beginParsing` /
//! `prepare` / `finalizeParsing`），G1 返回**诚实错误**并登记为临时缺口（见
//! `commands::import`）。
//!
//! # 已确认的刻意偏离（G1 开工清单 §2）
//!
//! | # | 偏离 | 理由 |
//! |---|---|---|
//! | D1 | 来源身份摘要改为**无密钥 `sha256`**（基线为 `HMAC(applicationSecret, …)`） | 身份从不进入渲染层；零新依赖 |
//! | D2 | 不迁移 `webContentsId` 归属校验（`clear()` 退化为「清空全部待处理检视」） | Tauri 单窗口，无多 webContents 语义 |
//! | D3 | 不建 `import_legacy_identity_bridge`，legacy 解析路径整体跳过 | L3 双栈项目目录刻意不互通 |
//! | D4 | `.epub` 暂不支持（对话框仍列出，选中后返回诚实错误） | Rust 解包需 `zip` 类新依赖（Ask first） |
//! | D5 | 文件读取直接走 `std::fs`（带字节上限），不引入基线句柄链 | 路径从不回传渲染层；上限与基线一致 |
//! | D6 | 对话框用 `tauri-plugin-dialog`（`pick_files` + 筛选器 + 多选） | 与 `dialog:select-knowledge-files` 同路径 |
//! | D7 | （本批新增）来源文件按 **数字感知自然序** 排序，逼近但不等价于 `localeCompare(…, 'zh-CN', {numeric:true})` | 完整 zh-CN 拼音排序需 ICU 类新依赖（Ask first）；章号最终由正文/文件名解析决定，排序只影响来源处理顺序 |

pub mod batch_checkpoint;
// 批次 G2b：来源身份解析（无密钥版 resolveEncodedSources）
pub mod identity;
pub mod inspection_store;
pub mod limits;
pub mod parsing;

use serde::{Deserialize, Serialize};

/// 导入用途 —— 对齐 `src/shared/import-run.ts` 的 `ImportPurpose`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportPurpose {
    #[serde(rename = "reference")]
    Reference,
    #[serde(rename = "author-manuscript")]
    AuthorManuscript,
}

impl ImportPurpose {
    /// 契约字符串（`ipc-channels.ts` / 持久化 `import_runs.purpose` 同源）
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::AuthorManuscript => "author-manuscript",
        }
    }

    /// 从持久化 / 契约字符串还原（非法值返回与基线同义的错误）
    pub fn parse_contract(value: &str) -> Result<Self, String> {
        match value {
            "reference" => Ok(Self::Reference),
            "author-manuscript" => Ok(Self::AuthorManuscript),
            _ => Err("导入用途无效".to_string()),
        }
    }
}

/// 导入文案语言 —— 对齐 `ImportRunLocale`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportRunLocale {
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en-US")]
    EnUs,
}

impl ImportRunLocale {
    /// 契约字符串
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::EnUs => "en-US",
        }
    }

    /// 从持久化 / 契约字符串还原（非法值返回与基线同义的错误）
    pub fn parse_contract(value: &str) -> Result<Self, String> {
        match value {
            "zh-CN" => Ok(Self::ZhCn),
            "en-US" => Ok(Self::EnUs),
            _ => Err("导入运行语言无效".to_string()),
        }
    }
}

/// 作者原稿预览过期错误码（对齐契约 `AUTHOR_IMPORT_PREVIEW_STALE`）
///
/// D10：Rust 侧无异常类，用 `PrepareError::AuthorPreviewStale` + 本错误码表达；
/// 命令层据此回 `{ success:false, errorCode:'AUTHOR_IMPORT_PREVIEW_STALE' }`。
pub const AUTHOR_IMPORT_PREVIEW_STALE: &str = "AUTHOR_IMPORT_PREVIEW_STALE";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purpose_roundtrips_contract_strings_test() {
        assert_eq!(ImportPurpose::Reference.as_str(), "reference");
        assert_eq!(
            ImportPurpose::AuthorManuscript.as_str(),
            "author-manuscript"
        );
        assert_eq!(
            serde_json::from_str::<ImportPurpose>("\"author-manuscript\"").unwrap(),
            ImportPurpose::AuthorManuscript
        );
        assert!(serde_json::from_str::<ImportPurpose>("\"bogus\"").is_err());
    }

    #[test]
    fn locale_roundtrips_contract_strings_test() {
        assert_eq!(ImportRunLocale::ZhCn.as_str(), "zh-CN");
        assert_eq!(ImportRunLocale::EnUs.as_str(), "en-US");
        assert_eq!(
            serde_json::from_str::<ImportRunLocale>("\"en-US\"").unwrap(),
            ImportRunLocale::EnUs
        );
    }
}
