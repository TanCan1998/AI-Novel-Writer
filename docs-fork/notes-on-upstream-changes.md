# Fork 对上游 docs/ 的改动登记

> 本文件登记 TanCan1998/Lorekeeper 对上游 `docs/` 文件的**全部**改动。
> 按 [`README.md`](README.md) 的边界规则，fork **不修改** `docs/`；希望保留的改动记在这里，
> 合并上游时 `docs/` 直接接受上游版本（本文件的登记项会在合并中丢失，需按此清单重新施加）。
>
> **基准上游**：`992b3f5f40165b59be7f1c126166235e7a7f807e`（`upstream/master`，2026-10-01）
> **登记时间**：2026-10-09（随 `docs/` → `docs-fork/` 迁移）

---

## 1. `docs/README.md` —— 交接快照命名规则段落（**必须保留**）

**位置**：`## 新文档放置规则`（bullet 列表）中，「临时计划和交接使用日期化文件…」一行之后，
「不在文档中复制 API Key…」一行之前。

**拟插入内容**：

```markdown
- **交接快照按日期命名，一个工作日一个新文件**：`docs-fork/handoffs/YYYY-MM-DD-tauri-migration-status.md`；
  当日新增内容只写入当日文件，跨日不回填旧文件（旧文件为冻结快照，仅允许加前向指针）。
  完整规则与快照结构见 [`agents/pi-development.md`](agents/pi-development.md) §9。
```

**保留理由**：这是从上游 `docs/` 体系发现 fork 快照规则的入口。**但迁移后已不再必需** ——
入库的公共入口改由 [`docs-fork/README.md`](README.md) 承担；上游 `docs/` 应保持纯上游状态，
不得为了"指路"而再次修改。

**重新施加方式**：默认**不重新施加**（只在确需向上游读者指路时才考虑）。
若将来确要施加，插入后须重新跑 `git diff upstream/master -- docs/` 并接受该差异，
同时更新本节。

**迁移前后差异**：
- 迁移前（已删除）：路径写作 `docs/handoffs/…`，第 3 行链接为 `agents/pi-development.md`
  （相对 `docs/` 解析，迁移后该目标已不存在）。
- 迁移后：路径改为 `docs-fork/handoffs/…`，链接改为指向 `docs-fork/agents/pi-development.md`。

---

## 2. `docs/adr/0006-unified-cross-platform-github-release.md` —— **已放弃**

fork 曾在该文件顶部添加 3 行 `Status: Superseded` / `Superseded-by: 0016` 抬头。

**处置**：**决定放弃**（2026-10-09）。理由：
1. 上游该文件正文第 3 行已声明「已由 0016 取代」，信息不丢失；
2. 上游 `docs/README.md` §「已取代的决定」已登记该取代关系；
3. fork 侧的状态可写在本目录的 [`adr/README.md`](adr/README.md) 索引中，无需改上游文件。

**结论**：不再登记为待保留改动。`docs/` 中的该文件保持上游原样。

---

## 3. `docs/plans/chapter-goal-review-production.md` —— **无内容改动**

fork 曾将其 rename 为 `2026-10-08-chapter-goal-review-production.md`（blob 与上游逐字节相同，
`git diff -M` 报 `rename (100%)`），后又未提交地移入 `archive/`。

**处置**（2026-10-09 迁移时）：撤回未提交的 `archive/` 移动，删除 rename 结果，
`git checkout upstream/master` 还原为上游原名。

**结论**：**内容零改动**，无需登记。该文件现为纯上游状态。

---

## 维护提醒

- 合并上游后跑 `git diff upstream/master -- docs/`；**期望无输出**。
- 若出现差异，说明有人误改了 `docs/`：应把改动迁到 `docs-fork/` 并在此登记，
  而不是接受该差异。
- 本文件自身属于 fork 文档，可自由更新。