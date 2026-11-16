# 交接文档

更新：2026-11-15。给接手的人（或下一个会话的我）看的状态说明。

## 1. 项目概况

- 仓库：`Joyi-code/grids`，Windows 多窗格文件管理器，界面名 **文件看板 / PaneBoard**，可执行文件 `grids.exe`。
- 技术：Rust + GPUI（`vendor/gpui` 本地补丁版），`crates/core` 纯逻辑、`crates/shell` Win32、`src/` 界面。
- 分支：`main`（只由 PR 合并）与 `gpui-rewrite`（工作分支）。不要合并上游 `efww/grids`。
- 版本号：固定在 `0.9.x`（0.9.10、0.9.11……），不主动跳到 1.0.0。

## 2. 代码状态

- 已发布版本 **v0.9.13**（tag `v0.9.13`）：资产 `grids-0.9.13.exe`，9,750,016 字节，SHA256 `493C8DE54DEE53F35C11EF53C41F2010F739086E6668FE72C111CAFE589044A2`。代码内容 = `49aa181`（拖到外部、滚动条、浮窗补齐）+ `Version 0.9.13` 版本号提交。
- 上一版 `v0.9.12`、`v0.9.11`、`v0.9.10` 已按政策转成草稿；公开的只有 v0.9.13。
- 本地 `main` 与 `origin/main` 同步，工作区干净。
- 最近提交：

```
b569a30 app: closing the window hides it in the notification area
2356a15 docs: handoff for v0.9.11
97e50e6 docs: handoff after the push
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
- **浮窗（0.9.11 新增，标题栏已图标化）**：每个窗格标题栏有外链图标，弹出独立窗口；浮窗里的窗格在原来的链接图标位置显示图钉（置顶）和恢复图标，没有独立顶栏，也没有透明度；窗口用自绘标题栏（文件夹名 + 窗口按钮），不显示系统标题栏。
- **关闭即隐藏（0.9.12 新增）**：主窗口点关闭不退出，收进右下角通知区；托盘图标单击恢复窗口，右键菜单「显示主窗口」「退出」，退出前先保存会话。浮窗点关闭仍然只关那一个窗口。Windows 专属；其他平台或托盘创建失败时保持关闭即退出。
- **拖到外部程序（0.9.13 新增）**：按住文件拖出窗口后改用系统的 OLE 拖放（`grids_shell::drag_files`，数据对象是 shell 的 IShellItemArray → IDataObject），可以拖进聊天软件、桌面、资源管理器等。只做复制（`DROPEFFECT_COPY`），不移动、不删除源文件。窗口内的拖动仍走内部逻辑（同盘移动、拖进文件夹、只读文件夹只复制）。主窗口和浮窗都适用；浮窗拖出去时会经过主窗口，2026-11-16 修掉了那条路径上的崩溃（见第 8 节）。
- **浮窗补齐单格功能（0.9.13 新增）**：浮窗现在自己应答窗格事件——删除走和宫格同一张确认卡（`src/confirm.rs`，浮窗内居中显示），删除后刷新本格并让宫格其它格跟着刷新，提示信息显示在浮窗底部，排序/列宽/收藏等改动交回 `Workspace::note_changed` 保存。**移动**（拖进文件夹、剪切粘贴）也补齐了：移动前要过占用检查（`PaneEvent::Lockers`），浮窗后台跑 `grids_shell::lockers` 后直接继续，被占用时先给一条提示；菜单里的「查看占用」在浮窗改成用提示条报占用者。**撤销（Ctrl+Z）**：浮窗根元素挂 `Float` 键位上下文，直接调用 `history::next_undo` / `history::apply_undo`（这两个从宫格的历史面板抽出来共用），危险情形先弹同一张确认卡。**文件夹备注**：`src/note_card.rs` 是宫格与浮窗共用的备注卡，浮窗保存后通过 `Workspace::note_saved` 存盘并刷新两边的备注行。浮窗窗格的 `active = true`，选中行配色和宫格一致。宫格专属的东西在单格浮窗里刻意不做：联动与对比、今日、命令面板、会话、布局、在下一格打开（菜单项在浮窗里隐藏）。
- **只开一个实例（未发版）**：`crates/shell/src/instance.rs` 用命名互斥量 `Grids.SingleInstance.v1` 保证单实例。第二次启动会找正在运行实例的隐藏窗口（类名 `GridsInstanceWindow`）并发 `WM_APP+2`，让它的窗口显示并置前（最小化会还原，收在托盘也能叫回来），自己随即退出；任务栏因此只有一条。启动顺序：`update::apply_from_args` 之后、打开窗口之前 `instance::claim()`；窗口建好后 `instance::watch(..)`，事件走和托盘同一条通道。
- **滚动条（0.9.13 新增）**：文件列表右侧有细滚动条，内容超出时才出现；拖滑块定位、点轨道跳转（Explorer 式），滚轮滚动时滑块跟着走。实现见 `src/pane.rs` 的 `Sbar` / `sbar_*`，用 `ScrollHandle::set_offset` 直接写滚动位置。

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
- 透明度滑块按用户要求删除，`grids_shell::window::set_alpha` 一并移除。
- 浮窗不显示系统标题栏，改成自绘标题栏：文件夹名 + 最小化/最大化/关闭，配色跟主窗口和主题的明暗一致；任务栏和 Alt+Tab 显示「文件看板 · 文件夹名」。

### 实现

- `src/pane.rs`：`FloatCtl { pinned: Rc<Cell<bool>>, workspace, index }`，字段 `PaneView.float`；`toggle_pin` 用 `ui::hwnd(window)` 调 `set_always_on_top`；`restore_from_float` 组 `PaneSave` → `workspace.adopt_float(index, save, cx)` → `window.remove_window()`。
- `src/float.rs`：`FloatPane` 只剩 `Entity<PaneView>` 和观察它的订阅；`bar()` 画自绘标题栏（`title_bar()` 负责拖动，`ui::window_controls` 出窗口按钮）。
- `src/workspace.rs`：`detach_pane` 的 `WindowOptions` 用 `appears_transparent: true` 去掉系统标题栏，`window_decorations` 只在 Linux 用 Client。
- `src/assets.rs`：新增 `icons/pin.svg`（Lucide pin）。
- 浮窗里的窗格按钮常显：`quiet` 额外要求 `self.float.is_none()`。
- 浮窗标题栏双击不再发 `ToggleMaximize`。

### 验收（2026-11-15 本机实测）

1. 浮窗无独立顶栏，控件在标题栏。
2. 浮窗内无外链图标。
3. 图钉点击后窗口 `WS_EX_TOPMOST` 置位，再点清除。
4. 恢复后主窗口对应窗格变成浮窗当前目录与视图，浮窗关闭。
5. 用自绘标题栏的关闭按钮（或系统快捷键）关闭浮窗，主窗口不变。
6. 自绘标题栏能拖动窗口，最大化按钮生效；深色、浅色主题都跟着变。

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
- **GPUI 里读 CF_HDROP 的 bug（2026-11-16，已修在 vendor）**：`WindowsDragDropHandler::DragEnter` 把 `hGlobal.0` 当指针解引用再去 `DragQueryFileW`，只有资源管理器那种 moveable 全局内存的布局才碰巧成立；shell 数据对象给的是 fixed 块，于是读到野指针，崩在 `windows.storage.dll`（0xc0000005）。现改成 `GlobalLock` 后把锁住的块当 HDROP 用，并在每条出口 `ReleaseStgMedium`。现象是「从浮窗往桌面拖就卡死/崩溃」，拖过任何 Grids 自己窗口都会触发。


