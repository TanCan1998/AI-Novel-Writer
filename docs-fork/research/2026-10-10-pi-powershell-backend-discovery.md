# pi 的 PowerShell 后端发现缺陷：Store 版 pwsh 7 被静默忽略（2026-10-10）

## 现象

本机已装 PowerShell 7（MSIX / Store，`7.6.6.0`），`where pwsh.exe` 能找到它，
`pwsh -Command '$PSVersionTable.PSVersion'` 也报 7.6.6；但 pi 的 `powershell` 工具实际跑的是
**Windows PowerShell 5.1**（工具内实测 `$PSVersionTable.PSVersion = 5.1.26100.9549`）。

## 根因

`@earendil-works/pi-coding-agent/dist/utils/shell.js`：

```js
export function getPowerShellConfig() {
    const shell = findExecutableOnPath("pwsh.exe") ?? findExecutableOnPath("powershell.exe");
    ...
}
function findExecutableOnPath(executable) {          // win32 分支
    const result = spawnSync("where", [executable], { encoding: "utf-8", timeout: 5000, windowsHide: true });
    if (result.status === 0 && result.stdout) {
        const firstMatch = result.stdout.trim().split(/\r?\n/)[0];
        if (firstMatch && existsSync(firstMatch)) return firstMatch;   // ← 卡在这里
    }
    return null;
}
```

Store/MSIX 版 PowerShell 的 `pwsh.exe` 是 **0 字节 App Execution Alias**
（`attributes = Archive, ReparsePoint`）。`where` 返回它，但：

| 检查 | 结果 |
|---|---|
| `where pwsh.exe` | status 0，第一行 = `C:\Users\<user>\AppData\Local\Microsoft\WindowsApps\pwsh.exe` |
| `existsSync(该路径)` | **false** |
| `statSync(该路径)` | **EACCES: permission denied** |

→ `findExecutableOnPath("pwsh.exe")` 返回 `null` → 回退 `powershell.exe`（5.1）。

**影响面**：任何通过 Store/MSIX 安装 PowerShell 7 的 Windows 机器，pi 都会**静默**回退到 5.1，
无任何提示。MSI 版（装到 `C:\Program Files\PowerShell\7\`，真实文件）不受影响。

## 可用 workaround

1. **改用户 PATH**（无需管理员、无需安装）：MSIX 包目录里的 `pwsh.exe` 是**真实文件**
   （`Test-Path` / `existsSync` 均为 true，直接启动报 7.6.6），把该目录
   `C:\Program Files\WindowsApps\Microsoft.PowerShell_<版本>_x64__8wekyb3d8bbwe`
   前置到**用户** PATH，`where` 的第一行就会变成真实文件。
   代价：目录名带版本号，PS7 升级后需更新一次。
2. **卸 MSIX 装 MSI 版**：winget 会写**机器级** PATH，而机器 PATH 排在用户 PATH 之前。

## 为什么「不改后端、只改写法」往往更划算（实测）

同一条失败命令（`git log --nope`）在三种写法下的上下文占用：

| 方案 | 字节 / 行数 |
|---|---|
| PS 5.1 + 裸写 `2>&1` | 293 B / 8 行 |
| PS 5.1 + `2>&1 \| ForEach-Object { "$_" }` | **38 B / 2 行** |
| pwsh 7 + `2>&1` | ~49 B / 1 行 |

即：**改写法比换后端更省**。（另：pwsh 7 不再把 stderr 包成 ErrorRecord，所以内容型过滤在它上面才真正可用。）

## 附：一个被实测证伪的「优化」（避免后人重走）

用 `2>&1 | Where-Object { $_ -notmatch 'CategoryInfo|FullyQualifiedErrorId|^\+ |所在位置' }`
**过滤不掉装饰行**：实测 8 行 / 329 字符（比裸写还多）。

原因：PS 5.1 里 `2>&1` 把原生命令的 stderr 变成 **ErrorRecord 对象**，`ToString()` 只返回消息本身；
那 7 行装饰是**渲染期**由格式化器添加的，内容过滤无从下手。
正确的做法是在渲染之前字符串化：`2>&1 | ForEach-Object { "$_" }`。

## 复现命令

```powershell
$pkg = (Get-AppxPackage Microsoft.PowerShell).InstallLocation + '\pwsh.exe'
Test-Path $pkg                                                           # True
node -e "console.log(require('fs').existsSync(process.argv[1]))" $pkg    # true
where.exe pwsh.exe                                                       # 第一行 = WindowsApps 别名
$PSVersionTable.PSVersion                                                # 在 pi 的 powershell 工具里 = 5.1
```

## 实测补充（2026-10-10）：本机 winget 会装回 MSIX，MSI 只能手工装

- `winget install --id Microsoft.PowerShell -e --source winget --silent` → **exit 0，但装回的仍是 MSIX**：
  `Get-AppxPackage` 又变回 present，`InstallLocation = C:\Program Files\WindowsApps\Microsoft.PowerShell_7.6.6.0_x64__8wekyb3d8bbwe`；
  而 `C:\Program Files\PowerShell` 无目录、`%LOCALAPPDATA%\Programs\PowerShell` 不存在、WinGet Packages 无落点。
- 先 `Remove-AppxPackage`（成功，`appx_after=gone`）再 `winget install` → **回到原状态**（无损害、也无进展）。
- 结论：本机要让 pi 用上 pwsh 7，**只能走「用户 PATH 前置包目录」**（上文 workaround 1），
  或手工下载 MSI + `msiexec /i`（需 UAC；`curl` 下载在本机失败且报错被环境的输出抑制吞掉，未能诊断）。
- 干跑验证（仅改 `$env:PATH`、不碰注册表）：`where pwsh.exe` 第一行变为包内真实 `pwsh.exe`，
  `existsSync(first) = true` ⇒ pi 的 `getPowerShellConfig()` **会返回 pwsh 7**。
- ⚠️ 若真的写 PATH：必须保持 `REG_EXPAND_SZ`（否则会破坏 PATH 中 `%VAR%` 的展开）；
  且目录名带版本号，**PS7 每次升级都会失效**。

## 成本收益结论（为什么最终没做）

| 方案 | 字节 / 行（同一条出错命令） |
|---|---|
| PS 5.1 + 裸写 `2>&1` | 293 B / 8 行 |
| PS 5.1 + `2>&1 \| ForEach-Object { "$_" }` | **38 B / 2 行** |
| pwsh 7 + `2>&1` | ~49 B / 1 行 |

**改写法（38 B）严格优于换后端（~49 B）** ⇒ 为 token 而切 pwsh 7 不成立；
切它的价值仅在「pwsh 7 不再把 stderr 包成 ErrorRecord，内容型过滤才真正可用」。
