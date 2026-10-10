# 开工清单：验证子代理编排（PiDeck 子代理面板 + `lorekeeper-task`）

> 日期：2026-10-10 ｜ 前置会话主题：「让 PiDeck 捕捉到派生的子任务」
> **本清单是给重启后的新会话看的**（前一会话上下文已断）。
> **cwd 必须是 `F:\workplace\pi\ai-novel-writer`**，否则 `.pi/agents/` 与项目 `AGENTS.md` 都不会生效。

## 1. 为什么要做这件事

前一会话调查了「在终端里 `pi -p` 派生子任务时，PiDeck 界面完全看不到」的问题。结论：

- **shell 派生是死路**。PiDeck 的桥接扩展（`D:\Program Files\PiDeck\resources\extensions\pi-deck-subagents.ts`）运行在**父 pi 进程内**，只订阅两类信号：
  1. `pi-subagents` 插件经 `pi.events` 广播的 `subagents:created/started/completed/failed/steered`
  2. 工具执行事件里工具名为 `acp_delegate` / `acp_delegate_cancel` 的（`billion-context-pi`）
  shell 启动的子进程是另一个进程，其 `pi.events` 父进程永远看不到。
- **可行路线**：装 `@tintinweb/pi-subagents`，改用它的 **`Agent` 工具**在**同一进程内**派发子代理。

## 2. 已经做完的准备工作（勿重复）

| 项 | 状态 |
|---|---|
| 安装 `npm:@tintinweb/pi-subagents` | ✅ `0.20.0`，已写入 `~/.pi/agent/settings.json` 的 `packages` |
| 扩展加载验证 | ✅ 新进程已注册 `Agent` / `SubagentWorkflow` / `get_subagent_result` / `steer_subagent` |
| 创建项目级 agent 类型 | ✅ `.pi/agents/lorekeeper-task.md`（`.gitignore:59` 已有 `.pi/` 规则，不会污染仓库） |
| agent 解析验证 | ✅ `Agent` 工具描述中出现 `lorekeeper-task`（`ling-3.1-flash`）（`Tools: read, edit, write, bash`） |
| 会话落盘机制验证 | ✅ 子 pi 写 session 文件后，PiDeck **6 秒内**自动登记进 `%APPDATA%\PiDeck\session-catalog.json`；删除文件后 PiDeck 也自动清理孤儿条目 |
| 实验残留 | ✅ 已清理（验证会话文件已删） |

## 3. 待验证（4 项，这是本轮的唯一目标）

1. **PiDeck 子代理面板是否实时出条目** —— 派发后应在界面出现 queued/running 条目。
2. **`toolUses` / `tokens` 是否实时跳动**。
3. **子代理的 `bash` 工具在 Windows 上能否真正执行命令**（**最高风险项**，见 §4）。
4. **`subagent_type: "lorekeeper-task"` 能否正常派发并按其系统提示词行事**。

## 4. 已知风险与对策

实测确认的工具名事实（来自 `pi-coding-agent/dist`，platform = win32）：

```
createCodingTools(".") → read, bash, edit, write
createReadOnlyTools()  → read, grep, find, ls
BUILTIN 白名单          → read, bash, edit, write, grep, find, ls   ← 没有 powershell
```

⚠️ 两个坑：
- **`tools:` 写 `powershell` 会被静默丢弃**（`csvList(val, BUILTIN_TOOL_NAMES)` 过滤），若该字段因此变成空列表 → **子代理零工具**。
- 本机 settings 是 `defaultTools: ["-bash", "+powershell"]`，**bash 在主会话被禁用**。子代理走自己新建的工具实例，是否受此约束、以及 pi 的 bash 在 Windows 上用什么 shell 执行，**未验证**。

**若 bash 不能跑命令** → 把 `.pi/agents/lorekeeper-task.md` 的 `tools:` 改为 `read, edit, write, grep, find, ls`（放弃子代理自检），**所有验收命令由编排者（主会话）执行**。这也符合既有的「编排者独立复验，不采信子代理自述」纪律。

## 5. 验证方法

用一次**真实但极小**的派发，同时覆盖 4 项：

```
Agent(
  subagent_type: "lorekeeper-task",
  name: "probe-1",
  description: "验证子代理通道",
  prompt: "只做一件事：用 bash 工具执行 `git rev-parse --short HEAD`，把原始输出贴出来。
           不要改任何文件，不要跑其他命令。"
)
```

- 保持 `run_in_background` 为默认（`true`）→ 才会发 `subagents:created`。
- 派发后请用户看一眼 PiDeck 界面（子代理面板 / widget）。
- 用 `get_subagent_result(agent_id, wait: true, verbose: true)` 取结果。
- 结果判读：
  - 能看到 HEAD → bash 可用，`lorekeeper-task` 可端到端自主自检。
  - 报工具不存在 / 命令执行失败 → 按 §4 对策改 `tools:`。

## 6. 参考路径

| 用途 | 路径 |
|---|---|
| PiDeck 子代理桥接源码 | `D:\Program Files\PiDeck\resources\extensions\pi-deck-subagents.ts` |
| PiDeck 会话目录索引 | `%APPDATA%\PiDeck\session-catalog.json` |
| PiDeck 项目注册表 | `%APPDATA%\PiDeck\projects.json` |
| subagents 插件源码 | `C:\Users\tanca\.pi\agent\npm\node_modules\@tintinweb\pi-subagents\` |
| 插件 README（Frontmatter 字段全表在 §Custom Agents） | 同上 `README.md` |
| 本项目 agent 定义 | `.pi/agents/lorekeeper-task.md` |

## 7. 本轮的编排纪律（沿用，勿改）

**编排者独立复验，不采信子代理自述。** 子代理报「测试通过」只是线索；编排者必须用自己的终端重跑原始验收命令，并核对：
- 所有命令 exit 0 且原始输出符合预期
- 测试**数量增加**（不是"总数没变但说加了测试"）
- `git status` **只有 Target 文件被改**
- `docs/` 上游镜像零改动；无新增依赖；未碰 `schema.rs`

---

## 8. 验证结果（2026-10-10 执行）

派发方式：`Agent(subagent_type: "lorekeeper-task", name: "probe-1", run_in_background: true)`，提示词即 §5 原文（未改一字）。

| §3 待验项 | 结论 |
|---|---|
| ① PiDeck 子代理面板实时出条目 | ✅ 通过（用户界面确认出现 `probe-1` 条目） |
| ② `toolUses` / `tokens` 实时跳动 | ⚠️ **未观察到**（用户未确认界面跳动；后端统计有值：`Tool uses: 1` / `23.7k token`） |
| ③ 子代理 `bash` 在 Windows 可执行 | ✅ **通过**（`git rev-parse --short HEAD` → `c0a0c5c2`，与编排者终端输出逐字一致） |
| ④ `subagent_type: "lorekeeper-task"` 端到端 | ✅ **通过**（25.9s / 1 tool use，按其系统提示词行事、零文件改动） |

- **§4 兜底方案不触发**：`bash` 可用 → `.pi/agents/lorekeeper-task.md` 的 `tools:` 维持原样（含 `bash`），子代理可自主自检。
- 编排者独立复验：`git status` 无新增改动（子代理零写入）；`.pi/` 由 `.gitignore:59` 忽略，未污染仓库。
- 遗留：② 的界面跳动未取得界面证据，如需补证可再派一次探针并在派发瞬间盯面板。
