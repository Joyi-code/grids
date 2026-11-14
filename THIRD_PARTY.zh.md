# 第三方代码与素材

本软件以 MIT 许可发布（见 `LICENSE`），包含或改编了以下内容。以下说明为中文参考，英文原文见 `THIRD_PARTY.md`。

## 应用图标

`assets/icon/` 下的图标由 AI 为本项目生成，随本项目的 MIT 许可发布，替换了更早的 `app.ico`。

## quiet-md（MIT）

版权所有 (c) 2026 s1dashu。https://github.com/s1dashu/quiet-md

改编内容：主题色板与深色模式配色（`src/theme.rs`）、通用界面组件（提示气泡、弹层外壳、菜单行、标题栏行为、窗口按钮、单行省略文本，`src/ui.rs`）、`vendor/` 下对 GPUI 与 taffy 的本地补丁（见 `vendor/PATCHES.md`）、工作区的 clippy 设置与 `rust-toolchain.toml`。未使用 quiet-md 的图标与名称。

```
MIT 许可证

版权所有 (c) 2026 s1dashu

特此免费授予任何获得本软件及相关文档文件（以下称「本软件」）副本的人不受限制地处理本软件的权利，包括但不限于使用、复制、修改、合并、发布、分发、再许可和/或销售本软件副本的权利，并允许获得本软件的人在遵守以下条件的前提下这样做：

上述版权声明和本许可声明应包含在本软件的所有副本或重要部分中。

本软件按「原样」提供，不附带任何明示或暗示的担保，包括但不限于对适销性、特定用途适用性和非侵权性的担保。在任何情况下，作者或版权持有人均不对任何索赔、损害或其他责任负责，无论该责任源于合同、侵权行为还是其他原因，也无论是否与本软件或本软件的使用或其他交易有关。
```

## 系统组件（不随程序分发）

`grids.exe` 是自包含的：Rust 标准库与 Microsoft C 运行库（vcruntime 与通用 C 运行库）以静态方式链接（`.cargo/config.toml`，`+crt-static`），因此不需要安装 Visual C++ 运行库。若 exe 导入了 `vcruntime*`、`msvcp*`、`api-ms-win-crt-*` 之类的 DLL，或导入了任何网络 DLL，CI 会直接失败。

它仍然导入的每个 DLL 都是 Windows 自带的。决定最低系统版本的是 `icuuc.dll`，即 Windows 自带的 ICU（Unicode，ICU 许可，Windows 10 1703 起内置）：GPUI 的 Windows 后端会调用其中的 `u_strlen`（`vendor/gpui/src/platform/windows/destination_list.rs`，经由 `windows` crate 的 `Win32_Globalization`）。加上 GPUI 使用的 DirectComposition 与 DirectWrite 接口，本程序要求 **Windows 10 1903 及以上，或 Windows 11，x64**。

## GPUI（Apache-2.0）

版权所有 Zed Industries, Inc.。https://github.com/zed-industries/zed

界面框架，使用 crates.io 上的 `gpui` 0.2.2，并在 `vendor/gpui` 做本地补丁（许可证文本见 `vendor/gpui/LICENSE-APACHE`）。`src/input.rs` 中的单行输入框改编自 GPUI 的 `examples/input.rs`（Apache-2.0）；改动包括：Grids 的按键上下文与绑定、占位文字、提交与取消事件、横向滚动与选择辅助函数。

## taffy（MIT）

GPUI 使用的布局引擎，在 `vendor/taffy` 做本地补丁，许可证在该目录内。

## Lucide 图标（ISC）

https://lucide.dev — `assets/icons/` 下的 SVG 图标来自 Lucide 0.468.0，描边宽度调整为 1.75。`folder-fill.svg` 与 `star-fill.svg` 是 Lucide `folder` 与 `star` 的实心版本。

```
ISC 许可证

Lucide 部分内容的版权由 Cole Bemis 2013-2022 持有，作为 Feather 的一部分（MIT）。Lucide 其余内容的版权由 Lucide Contributors 2022 持有。

特此授予任何人为任何目的使用、复制、修改和/或分发本软件的权利，无论是否收费，前提是上述版权声明与本许可声明出现在所有副本中。

本软件按「原样」提供，作者不对本软件作出任何明示或暗示的担保，包括但不限于对适销性和特定用途适用性的担保。在任何情况下，作者均不对因使用本软件或与本软件的使用或其他交易有关的任何特殊、直接、间接或后果性损害，或任何损害（包括但不限于使用、数据或利润的损失，无论是否源于合同、过失或其他侵权行为）承担责任。
```

`github.svg` 是 GitHub 的标识，仅用于链接到项目的 GitHub 页面。

其他 Rust 依赖来自 crates.io，遵循各自的许可证（见 `Cargo.lock`）。
