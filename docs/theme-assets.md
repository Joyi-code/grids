# 主题素材规范

给美术同学的交付说明。按这份文档做出来的素材，可以直接进主题包，不用再改。

- 适用版本：0.9.x（文件看板 / PaneBoard）
- 设计基准：100% DPI 的像素尺寸；程序在 125%、150% 下按倍数放大，所以优先矢量
- 交付格式：一个 zip，目录结构见第 5 节

## 1. 现在有什么

- 界面颜色：浅色、深色两套配色，全部由代码里的色值决定，没有图片。
- 图标：89 个线性图标（Lucide 0.468 风格），24×24 视图框，描边 1.75，单色，运行时由主题着色。
- 应用图标：`assets/icon/` 下的 `grids.ico`（16 到 256）和 16 到 1024 的 PNG。
- 目前没有位图背景、没有插画。

主题包要做的，就是把上面这些换成另一套。

## 2. 要准备哪些素材

| 类别 | 是否必需 | 文件名 | 格式 | 尺寸 | 说明 |
| --- | --- | --- | --- | --- | --- |
| 应用图标 | 必需 | `app.ico` + `app.png` | ICO + PNG | 多尺寸 + 1024×1024 | 圆角方形，内容占 88%，四周留 64px 安全边 |
| 侧栏标识 | 可选 | `brand.svg` | SVG | 24×24 视图框 | 不提供就用应用图标；显示只有 20×20，线条别太细 |
| 工具栏图标 | 可选 | `icons/*.svg` | SVG | 24×24 | 整套 89 个，见附录；也可只覆盖其中几个 |
| 文件类型图标 | 可选 | `types/*.svg` | SVG 或 PNG | 24×24（或 96×96 PNG） | 12 个：文件夹、文件、文本、表格、演示、图片、视频、音频、压缩包、代码、程序、设置 |
| 空状态插画 | 可选 | `empty.svg` | SVG 或 PNG | 240×160 | 文件夹为空时显示，透明底 |
| 窗口背景 | 可选 | `background.png` | PNG/JPG | 1920×1080 | 会被文件列表大面积覆盖，尽量低对比，≤400 KB |
| 关于页横幅 | 可选 | `about.png` | PNG | 640×120 | 许可证弹层顶部，透明或纯色底 |

只换配色、不换图标也可以，这时只需要第 4 节的色板。

## 3. 技术规范

SVG：

- SVG 1.1，`viewBox="0 0 24 24"`，不要写死 `width`/`height`
- 单色图标用 `stroke="currentColor"` 或 `fill="currentColor"`，程序会按主题染色
- 描边 1.75，`stroke-linecap="round"`，`stroke-linejoin="round"`
- 不要用 `<filter>`、`<image>`、`<text>`（文字转路径）、外部字体、外链、base64
- 单文件 ≤ 8 KB，路径合并

PNG：

- 透明底，sRGB，非交错，24/32 位
- 不要预乘 alpha，不要投影

ICO：

- 至少包含 16、24、32、48、64、128、256 七种尺寸

命名：

- 小写英文加连字符，例如 `file-spreadsheet.svg`，不要中文、空格、大写

深浅色：

- 同一份图标通用于浅色和深色，颜色由程序决定
- 如果某个图标必须自带颜色（例如彩色文件类型图标），请同时给 `xxx.svg` 和 `xxx-dark.svg`

## 4. 配色

即使不做图标，也建议提供一套配色。每个颜色给十六进制值，浅色、深色各一份。

| 字段 | 用在哪里 | 浅色示例 | 深色示例 |
| --- | --- | --- | --- |
| `background` | 窗口底、侧栏底 | #F7F8FA | #1C1C1E |
| `surface` | 文件列表、卡片 | #FFFFFF | #232326 |
| `text` | 正文 | #1F2225 | #E8E8EA |
| `text_caption` | 次要文字、日期 | #737477 | #9A9CA0 |
| `line` | 分隔线、边框 | #00000014 | #FFFFFF1A |
| `accent` | 选中、主按钮、链接 | #3A85F7 | #5D99F8 |
| `selection` | 选中行底色 | #0000000D | #FFFFFF12 |
| `hover` | 悬停行底色 | #00000008 | #FFFFFF0A |
| `added` | 对比：仅此侧有 | #16A34A | #4FB477 |
| `changed` | 对比：有差异 | #D97706 | #D9A441 |
| `danger` | 删除、只读提示 | #D92D20 | #C4574F |

对比度要求：正文与背景 ≥ 4.5:1，图标与背景 ≥ 3:1。

## 5. 交付结构

```
主题名/
  theme.json          配色与元信息（第 4 节字段，name / author / version）
  app.ico
  app.png
  brand.svg
  icons/              需要覆盖的图标，用附录里的同名文件
  types/              彩色文件类型图标
  empty.svg
  background.png
  about.png
```

打包成 `主题名-theme-v1.zip`，里面附一个 `README.txt`，写明作者、描边宽度、网格大小和用到的字体。

## 6. 验收清单

1. 16×16 下图标仍能辨认（应用图标、工具栏图标都要看）
2. 深色背景上图标与背景对比度 ≥ 3:1
3. 没有滤镜、没有外部字体、没有外链
4. ICO 在 100%、125%、150% 缩放下不糊
5. 文件列表被大面积遮挡时，背景图不抢注意力
6. 命名与附录一致，程序可以直接按文件名替换

## 7. 附录：89 个图标

只做主题包时需要覆盖的工具栏与状态图标，文件名即 `icons/` 下的名字。

- 导航与窗口：`app-window` `arrow-left` `arrow-right` `arrow-up` `chevron-down` `chevron-left` `chevron-right` `chevron-up` `panel-left` `panel-left-close` `panel-left-open` `maximize-2` `minimize-2` `x` `minus`
- 文件与文件夹：`file` `file-text` `file-code` `file-image` `file-video` `file-audio` `file-archive` `file-spreadsheet` `file-lock` `file-cog` `folder` `folder-fill` `folder-open` `folder-plus` `folder-x` `folder-search` `folder-input` `presentation` `table-2`
- 操作：`copy` `clipboard-paste` `scissors` `pencil` `trash-2` `undo-2` `save` `download` `refresh-cw` `plus` `check` `ellipsis` `search` `filter` `power` `terminal` `text-cursor-input`
- 视图与布局：`layout-grid` `layout-list` `list` `list-checks` `grid-2x2` `square` `square-split-horizontal` `arrow-down-narrow-wide` `arrow-up-down` `arrow-up-narrow-wide` `arrow-right-left`
- 状态与标记：`lock` `lock-open` `star` `star-fill` `star-off` `eye` `eye-off` `info` `shield-alert` `octagon-x` `file-lock` `sticky-note` `inbox` `link` `unlink` `external-link` `locate` `history` `calendar-clock` `git-compare` `layers` `hard-drive` `house` `monitor` `image` `sun` `moon` `sun-moon`

其中 `folder-fill`、`star-fill` 是实心版本，其余是线性版本，描边规则相同。
