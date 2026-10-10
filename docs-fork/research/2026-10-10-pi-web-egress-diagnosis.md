# pi Web 能力「疑似限流」实为旧进程出网坏态（2026-10-10）

## 现象

`web_search` / `fetch_content` 长期返回 **0 结果**，`mcp__parallel__web_search` 报 `fetch failed`，
体感是「被限流」。

**坑点**：`pi-web-access` **不抛错**——工具层返回「成功信封」，错误塞在 `queries[].error`：

```json
{"responseId":"mv201uyozh2nil","queries":[{"query":"...","answer":"","error":"fetch failed",
 "provider":"parallel-mcp","results":[]}]}
```

→ 空结果 + 不报错 ⇒ 问题长期隐形。

## 实测（同一查询，只换 proxy）

| 组 | 结果 |
|---|---|
| 无 proxy（auto / parallel-mcp / duckduckgo / gemini 四家全试） | ❌ 全部 `fetch failed`、0 结果 |
| `socks5h://127.0.0.1:7891` | ✅ 5 条 |
| `http://127.0.0.1:7890` | ✅ 5 条 |
| `http://127.0.0.1:7891` | ❌ `Proxy fetch ... returned no HTTP`（7891 只支持 SOCKS，不能当 HTTP 用） |

**重启 pi 后，不带 proxy 的四组测试全部成功**（含 `mcp__parallel__web_search` 返回 24,186 字符）。

## 排除的假设

| 假设 | 反证 |
|---|---|
| 供应商限流 | 同一配额重启后连打 3 次全成功 |
| 缺 `proxy` 配置 | 无 proxy 重启后全通 |
| PiDeck / 系统代理未开 | `piProxyEnabled=false` + 系统 `ProxyEnable=0`，且 pi 进程**无任何**代理环境变量 |
| 上游端点不通 | 端点正常，返回真实结果 |

## 结论

**上一个 pi 进程的网络出口处于坏状态**（长会话累积：连接池 / DNS 缓存失效），
每次 `fetch` 都失败。**重启 = 重建连接池即恢复。** 这也是「长会话跑到后半程 Web 整体失效」的成因。

## 应急开关（仅在直连真的坏时用）

`~/.pi/agent/web-search.json` 增加 `"proxy"`：

```json
{ "geminiApiKey": "<...>", "provider": "parallel-mcp", "proxy": "socks5h://127.0.0.1:7891" }
```

⚠️ 平时**不要加**：它会新增一个依赖 Clash 的失败点（Clash 重启/换端口 → 搜索立刻全挂）。
`pi-web-access` 用 undici `fetch`，**不读** `HTTP(S)_PROXY` 环境变量（其文档明写），故只能走该字段。

## 复发时的诊断配方（约 30 秒）

1. 不带 proxy 搜一次 → 查返回里的 `queries[].error`（**别看工具层是否报错**）；
2. 直调 `tools.mcp__parallel__web_search({ objective, search_queries })` 对照（注意 `objective` 必填）；
3. 若两者都 `fetch failed` → 全域出网问题 → 重启 pi；仍不通再考虑应急 `proxy`。

## 附：`parallel` 的两条路径是同一上游

`mcp.json` 的 `parallel`（`https://search.parallel.ai/mcp`）与 `web-search.json` 的
`provider: "parallel-mcp"` **是同一 vendor、共享配额**，不是两条独立通道。
`mcp.json` 无 `exposure` 字段 → 走 pi 默认 `codemode` 曝光（`dist/core/mcp-servers.js:135`），
**工具定义成本≈0**；实测其返回比 `web_search` 更丰富（含摘要/引用），故**建议保留**。
