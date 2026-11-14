# 主题素材规范 v2

PaneBoard / 文件看板 0.9.x。这份文档说明程序**实际读取**哪些文件、怎么用、还缺什么。给美工时按第 2 节的清单交付即可。

## 1. 先看这条：SVG 只能单色，彩色请用 PNG

程序的界面用 GPUI 绘制，SVG 在渲染时会被染成一种颜色（主题的图标色）。所以：

- **单色线条图标**（工具栏、侧栏）用 SVG，`stroke="currentColor"`，程序会按主题上色。
- **彩色素材**（文件类型图标、插画、横幅、背景）用 PNG，程序原样绘制，不做染色。

v1 包里 `types/*.svg` 的彩色底块不会生效，程序用的是同目录的 `types/*.png`。

## 2. 交付清单

### 必需

| 文件 | 用途 | 规格 |
| --- | --- | --- |
| `theme.json` | 浅色/深色两套色板 | 见第 3 节 |
| `icons/*.svg` | 工具栏 89 个单色图标 | 24×24 视图框，描边 1.75，`currentColor` |
| `app-sizes/app-48.png` | 侧栏左上角标识 | 48×48，透明底 |
| `types/*.png` + `types/*-dark.png` | 文件列表图标 | **256×256**，透明底，主体占 80% |

### 强烈建议

| 文件 | 用途 | 规格 |
| --- | --- | --- |
| `about.png` / `about-dark.png` | 版本弹层顶部横幅 | 640×120，低对比，透明或纯色底 |
| `empty-2x.png` / `empty-dark-2x.png` | 空文件夹插画 | 480×320（显示 180×120） |
| `app.ico` | 任务栏与窗口图标 | 16/24/32/48/64/128/256 |
| `background.png` / `background-dark.png` | 窗口背景（接入中） | 1920×1080 或 512×512 无缝平铺，低对比 |

### 可选

| 文件 | 用途 | 规格 |
| --- | --- | --- |
| `brand.svg` | 侧栏标识的单色版本 | 24×24，`currentColor`；不给就用 `app-48.png` |
| `empty-search.png` | 搜索无结果时的插画 | 480×320 |
| `missing.png` | 联动对比中“对应文件夹不存在”的插画 | 480×320 |
| `sidebar-foot.png` | 侧栏底部小插画 | 240×120，透明底 |

## 3. `theme.json`

```json
{
  "name": "主题名（显示在主题列表里）",
  "author": "作者",
  "version": "1.0.0",
  "light": {
    "background": "#F7F1E7",
    "surface": "#FFFCF6",
    "text": "#44352D",
    "text_caption": "#6E5B4D",
    "line": "#44352D24",
    "accent": "#A34F26",
    "selection": "#EEDBC5",
    "hover": "#F3E6D6",
    "added": "#3F704E",
    "changed": "#885F19",
    "danger": "#B33E39"
  },
  "dark": { "…同上 11 个字段…" }
}
```

- `line` 用 8 位十六进制表示透明度（`#RRGGBBAA`）。
- 缺哪个字段就沿用内置颜色，不会报错。
- 深浅两套必须都给：程序会在跟随系统 / 浅色 / 深色之间切换，用的就是这两套。

## 4. 文件类型图标的映射

程序按类型找 `types/<名字>.png`，深色模式找 `types/<名字>-dark.png`：

| 类型 | 文件名 |
| --- | --- |
| 文件夹 | `folder` |
| 普通文件 | `file` |
| 文本 / Word / PDF | `file-text` |
| 表格 | `file-spreadsheet` |
| 演示文稿 | `presentation` |
| 图片 | `file-image` |
| 视频 | `file-video` |
| 音频 | `file-audio` |
| 压缩包 | `file-archive` |
| 代码 | `file-code` |
| 程序 | `app-window` |
| 设置类 | `file-cog` |

想给 Word / PDF / PPT 各自单独的形象，可以补 `file-word.png`、`file-pdf.png`、`file-slides.png`（含 `-dark`），程序会优先用它们；这需要我在代码里加三行映射。

## 5. 尺寸与清晰度

- 列表和详细信息里图标显示 **18×18**；中/大/超大图标视图分别是 48、96、192。
- 一图多用：**直接交 256×256 PNG**，程序会按需缩放。
- 18px 下要能认出：主体占画布 80%，最细的线不小于 2px（按 96px 画布算约 10px），不要依赖文字。
- 交付时把浅色、深色两套都放在同一目录，文件名只差 `-dark`。

## 6. 目录结构

```
cream-cat/
  theme.json
  app.ico
  app.png                   1024×1024
  app-sizes/app-48.png
  brand.svg
  about.png                 about-dark.png
  background.png            background-dark.png
  empty-2x.png              empty-dark-2x.png
  empty-search.png
  missing.png
  icons/                    89 个单色 SVG
  types/                    每种类型 256 PNG，浅深各一份
```

`previews/`、`manifest.json`、`validation.json` 只供审阅，程序不读取，可以保留。

## 7. 验收清单

1. 18px、48px、96px 三个尺寸下图标都能认出是什么类型
2. 深色背景上图标与背景对比度 ≥ 3:1
3. 彩色素材是 PNG，单色素材是 SVG 且用 `currentColor`
4. 深浅两套齐全，命名只差 `-dark`
5. 没有外部字体、滤镜、外链
6. `theme.json` 的 11 个字段两套都给

## 8. v1 包已经可用的部分

`cream-cat-workspace-theme-v1` 现在能直接跑：色板、89 个工具栏图标、48px 标识、12 种类型图标、空状态插画、关于横幅都生效。

还没生效的：`background*.png`（窗口背景尚未接）、`app.ico`（任务栏图标要改代码）、`brand.svg`（当前用 app-48.png）、`types/*.svg`（彩色 SVG 不生效，用 PNG）。
