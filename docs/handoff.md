# 交接文档

更新：2026-11-15。给接手的人（或下一个会话的我）看的状态说明。

## 1. 项目概况

- 仓库：`Joyi-code/grids`，Windows 多窗格文件管理器，界面名 **文件看板 / PaneBoard**，可执行文件 `grids.exe`。
- 技术：Rust + GPUI（`vendor/gpui` 本地补丁版），`crates/core` 纯逻辑、`crates/shell` Win32、`src/` 界面。
- 分支：`main`（只由 PR 合并）与 `gpui-rewrite`（工作分支）。不要合并上游 `efww/grids`。
- 版本号：固定在 `0.9.x`（0.9.10、0.9.11……），不主动跳到 1.0.0。

## 2. 代码状态

- 远端 `origin/main` = `58ef790`，对应已发布版本 **v0.9.10**。
- 本地 `main` 领先远端 **15 个提交**（到 `801c0b0` 为止），**没有推送**（用户要求推送前必须先征得同意）：

```
801c0b0 float: pin and restore icons in the pane header, no window bar
36e303a docs: handoff for the floating pane and the rest
bc5f9f0 pane: a detach button in every pane header
1592af5 float: a toolbar button for the floating window, a shorter slider
1ee0de7 float: a detached pane window with pin, alpha slider and restore
d2cbac6 docs: floating pane design, opacity as a 30-100% slider
3bdc4f6 shell: always-on-top and window alpha primitives
5e288c8 pane: the column separator follows a header hover
87e5c68 pane: the column separator shows on hover only
e707492 pane: a wider, visible column-resize handle
feb3d55 search: split a Chinese phrase into two-character terms
2fc960d search: ignore the names of folders above the search root
2a3bb4c Version 0.9.11
6450a6e search: Ctrl+F now searches the folder tree
42a7a17 search: matcher and folder walker for the deep search
```

- 本地已编译的最新程序：`D:\tmp\grids-dev\grids-0.9.11.exe`（配置与主题在同目录）。
- 工作区干净，没有未提交改动。

## 3. 已具备的功能

- 1–16 个窗格、13 种布局、拖动分隔线与双击等分、每格独立历史与列宽。
- 监视目录变化，新文件高亮 4 秒，标题旁 `+N` 计数累加。
- 只读锁定：整格遮罩，锁定后标题栏按钮全部变灰禁用，只有锁图标可用。
- 联动与对比、今日视图、暂存架、会话、命令面板（Ctrl+K）、撤销。
- 主题：`奶油猫咪`、`变形金刚`，编译进 exe，首启写入 `themes\`；主题选择在版本弹层，选完自动重启。
- 一键更新：下载 → SHA-256 校验 → 只替换 exe → 重启；配置、日志、主题目录不动。
- **目录树搜索（0.9.11 新增）**：Ctrl+F 递归搜索当前窗格目录，空格分词，中文按两字拆词，类型列显示命中的子目录。
- **浮窗（0.9.11 新增，标题栏已图标化）**：每个窗格标题栏有外链图标，弹出独立窗口；浮窗里的窗格在原来的链接图标位置显示图钉（置顶）和恢复图标，没有独立顶栏，也没有透明度。

## 4. 发布政策（重要）

- 只保留**最新版本**公开：发布新版后，把上一版转成草稿：

```powershell
$id = gh api repos/Joyi-code/grids/releases/tags/v0.9.10 --jq '.id'
gh api -X PATCH repos/Joyi-code/grids/releases/$id -F draft=true   # 必须是 -F，-f 无效
```

- 资产名 `grids-<版本>.exe`，一个文件，不加 zip。
- 版本号 0.9.x 递增，不跳 1.0.0。
- 提交身份用匿名邮箱（已设在本地仓库）：`像素与咖啡时光 <206511176+Joyi-code@users.noreply.github.com>`。
- 推送、发版、删除内容前必须先问用户。

## 5. 已完成：浮窗标题栏图标化

### 结果

- 浮窗没有自己的顶栏：浮窗就是一个窗格，控件在窗格标题栏里。
- 浮窗标题栏在链接图标的位置依次显示图钉（置顶）和恢复（把目录交还主界面），都是图标。
- 浮窗里的窗格不显示外链图标，不能再弹一层。
- 透明度滑块按用户要求删除，`grids_shell::window::set_alpha` 一并移除；窗口移动、关闭用系统原生标题栏。

### 实现

- `src/pane.rs`：`FloatCtl { pinned: Rc<Cell<bool>>, workspace, index }`，字段 `PaneView.float`；`toggle_pin` 用 `ui::hwnd(window)` 调 `set_always_on_top`；`restore_from_float` 组 `PaneSave` → `workspace.adopt_float(index, save, cx)` → `window.remove_window()`。
- `src/float.rs`：`FloatPane` 只剩 `Entity<PaneView>`，`bar()` 已删；创建后 `pane.set_float(FloatCtl { .. })`。
- `src/assets.rs`：新增 `icons/pin.svg`（Lucide pin）。
- 浮窗里的窗格按钮常显：`quiet` 额外要求 `self.float.is_none()`。
- 浮窗标题栏双击不再发 `ToggleMaximize`。

### 验收（2026-11-15 本机实测）

1. 浮窗无独立顶栏，控件在标题栏。
2. 浮窗内无外链图标。
3. 图钉点击后窗口 `WS_EX_TOPMOST` 置位，再点清除。
4. 恢复后主窗口对应窗格变成浮窗当前目录与视图，浮窗关闭。
5. 用系统标题栏关闭浮窗，主窗口不变。

## 6. 其他待办

- 搜索内容匹配：目前只匹配名称与相对路径。可选方案（用户未定）：读 txt/docx/xlsx/pptx 内容（需自写 zip 读取 + 去标签）、接 Windows 搜索索引。
- 搜索索引缓存：目前每次改词都重新遍历目录树。
- 任务栏与窗口图标固定，不随主题变化（用户明确要求）。
- 主题素材规范见 `docs/theme-assets.md`（v2），浮窗设计见 `docs/floating-pane.md`。

## 7. 本机环境

- 工具链：`$env:PATH="D:\codex\.cargo\bin;$env:PATH"`，然后 `cargo build --release`。
- 测试：`cargo test --release -p grids_core`，`-p grids`。
- 构建前先关掉正在运行的实例（进程名形如 `grids-0.9.11`，`Get-Process grids` 匹配不到）。
- 测试数据：`D:\mock\grids\`，配置 `D:\mock\config\{a,b-dark,c-locked,d-compare}.json`。
- 截图脚本：`D:\tmp\grids-verify\win.ps1`（按 PID 截窗口）、`D:\claude code\_gridsrun\ux.ps1`（Click/SetForegroundWindow）。
- 截图注意：窗口阴影会被截进去，需要裁掉左右各 7 像素、底部 8 像素。

## 8. 踩过的坑

- PowerShell 单行替换里用 `\n` 或 `` `n `` 拼接多行会写坏 Rust 源码；多行改动一律用 `edit` 工具。
- GPUI 的 SVG 只能单色染色，彩色素材必须用 PNG。
- 主题解包曾因「写出 theme.json 后跳过剩余文件」导致 `types\` 为空，已修，且现在只补缺失文件。
- 重写提交前先确认 `git branch --show-current`，曾多次误提交到 main。
- 单测：`crates/core/src/search.rs` 5 个、`src/update.rs` 1 个。
