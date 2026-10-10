# B24 整屏瞬黑调研报告

日期：2026-10-10 · 状态：根因未实锤，候选方案待验证

## 现象

窗口最小化/最大化后，鼠标在窗口内移动时**整屏瞬黑**（偶发）。浏览器打开同一页面拖动不闪。

## 已排除（快照第三十八次记录 + 本次复核）

- 透明/effect 配置（`tauri.conf.json` 无 transparent/effects）
- 常驻 `backdrop-filter`、`backgroundColor` 缺失（已设 `#1e1e1e`）
- resize 重渲染风暴（前端无任何 resize 监听；`TitleBar.tsx` 仅 `invoke` 窗口命令）
- `shadow:false`（实测无效已回滚）
- 事件日志无 TDR/dxgkrnl/DWM 错误（非驱动崩溃）

## 环境（高危因子）

AMD Radeon（2021-11-30 旧驱动）+ RTX 3060 Laptop 混合显卡；单屏 2560×1440@165Hz；FreeSync/VRR 开启；`decorations: false` 自绘标题栏。

## 本地核查

- Tauri 2.12.1 原生支持 `additional_browser_args(&str)`（`WebviewBuilder`/`WebviewWindowBuilder`，Windows-only；wry 默认已传 `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`，使用时需自行保留）。配置窗口在 `setup` 前已创建，注入参数需在 `setup` 中用 `WebviewWindow::builder` 重建主窗口。

## 上游核查（EthanYoQ/AI-Novel-Writer）

- 上游领先 fork 基线 `origin/master` 共 **502 提交**（fork 基线领先上游 131 迁移提交），**无任何闪烁/黑屏修复**（关键词零命中，issues #278–#340 无相关报告）。
- `electron/main.ts` 唯一窗口改动（`003dc9b3`）：`frame: false` → `frame: Boolean(startupBlockedCode)`，属「全局数据迁移启动门禁」功能，非防闪烁；正常启动仍 `frame: false`。
- 上游经典 `TitleBar.tsx` 仍保留 `startViewTransition` + `document.documentElement.animate({clipPath})` 根动画，与 fork 基线同源；v2 标题栏（无此动画）已被 `57538059` 删除。

## 根因推断（分析判断，非实锤）

`decorations:false` 下 WebView2 走 DirectComposition 视觉树合成；最小化/最大化使合成表面失效重建，鼠标移动触发呈现时，DWM 在 165Hz+VRR+MPO 环境下与 WebView2 表面交接出现一帧黑屏；旧 AMD 驱动 + 混合显卡加剧竞态。上游 Electron 也存在此问题，提示双栈共用层（自绘标题栏 + View Transitions clip-path 根动画）嫌疑上升。

## 候选方案（按侵入性排序）

1. **M3**：Windows 图形设置给 `lorekeeper.exe` 指定单一 GPU（零代码）
2. **M4**：临时切 60Hz，验证 165Hz+VRR 是否为触发因子（零代码）
3. **A2**：`setup` 中重建主窗口注入 WebView2 `--disable-direct-composition`（小改）
4. **M2**：更新 AMD 驱动
5. **M1**：关 MPO（注册表，需审批+重启）

定位手段：GPUView / WPR 抓 DWM present 帧，确认黑帧出现在合成交接时刻。
