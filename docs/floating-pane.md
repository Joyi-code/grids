# 浮动窗格（设计稿）

从主窗口弹出一个独立的小窗口，显示同一个目录，可以置顶。主窗口里原来的窗格保持不动。

## 已确定

- 浮窗与主窗格**各自独立**：两边导航互不影响（用户选定的方案 A）。
- 浮窗没有自己的顶栏；置顶用标题栏里的图钉图标，恢复用标题栏里的恢复图标（用户 2026-11-15 的定稿）。
- 浮窗不显示系统标题栏：自绘标题栏（文件夹名 + 最小化/最大化/关闭），与主窗口同款、跟随主题明暗。
- 置顶：`SetWindowPos(HWND_TOPMOST)`。
- 透明度：**不做**。用户 2026-11-15 明确去掉滑块，`set_alpha` 已删。
- 浮窗位置和大小记住，下次弹出恢复。
- 浮窗关闭用自绘标题栏的关闭按钮。

## 已具备

`crates/shell/src/window.rs`：

- `set_always_on_top(hwnd, on)`

`ui::hwnd(window)` 可以拿到任意窗口的 HWND。

## 当前实现

1. `src/float.rs`：`FloatPane` 持有 `Entity<PaneView>`，创建后调 `pane.set_float(FloatCtl { .. })`；`bar()` 画 48 像素自绘标题栏（文件夹名 + 窗口按钮，`title_bar()` 拖动）。
2. `src/pane.rs`：`PaneView.float = Some(FloatCtl)` 时，标题栏在链接图标的位置显示图钉与恢复两个图标；浮窗内不画外链图标。
3. 入口：窗格标题栏的外链图标，以及快捷键 `Ctrl+Shift+D` 弹出当前窗格。
4. 恢复：读浮窗窗格的 `path`、`sort`、`ascending`、`show_hidden`、`columns`、`view` 组成 `PaneSave`，交给主窗口的 `adopt_float`，然后关窗。

## 边界

- 浮窗是独立视图，主窗格切走目录，浮窗不跟随。
- 两个窗口各自有焦点，输入框、菜单等按窗口隔离。
- 浮窗关闭不影响主窗格，反之亦然。
