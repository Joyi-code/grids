# 浮动窗格（设计稿）

从主窗口弹出一个独立的小窗口，显示同一个目录，可以置顶、可以调透明度。主窗口里原来的窗格保持不动。

## 已确定

- 浮窗与主窗格**各自独立**：两边导航互不影响（用户选定的方案 A）。
- 置顶：`SetWindowPos(HWND_TOPMOST)`，标题栏图钉切换。
- 透明度：分层窗口 + `SetLayeredWindowAttributes`，**连续滑块 30%–100%**，拖动即时生效，半透明时仍可点击。
- 浮窗位置和大小记住，下次弹出恢复。

## 已具备

`crates/shell/src/window.rs`：

- `set_always_on_top(hwnd, on)`
- `set_alpha(hwnd, percent)`（钳制到 30–100）

`ui::hwnd(window)` 可以拿到任意窗口的 HWND。

## 待做

1. `src/float.rs`：`FloatPane` 实体，持有 `Entity<PaneView>`、`hwnd`、`pinned`、`alpha`、`FocusHandle`，渲染顶部条（图钉、滑块、关闭）加窗格本体。
2. 入口：窗格右键菜单「弹出为浮动窗口」，以及快捷键 `Ctrl+Shift+D` 弹出当前窗格。
3. `workspace.rs`：`cx.open_window(WindowOptions { window_bounds, titlebar, .. }, ...)`，标题「文件看板 · 浮窗」。
4. 滑块：`on_drag` 或按下后跟随鼠标，拖动时调用 `set_alpha` 并记住值。
5. 配置：在 `Config` 里加 `float: Option<FloatBounds>`（位置、大小、置顶、透明度），浮窗关闭时写回。
6. 忽略：非 Windows 平台不启用置顶与透明度。

## 边界

- 浮窗是独立视图，主窗格切走目录，浮窗不跟随。
- 两个窗口各自有焦点，输入框、菜单等按窗口隔离。
- 浮窗关闭不影响主窗格，反之亦然。
