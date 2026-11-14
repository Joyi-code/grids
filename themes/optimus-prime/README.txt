变形金刚 钢铁领袖 · 完整主题包 v1.0.0

交付目标
按用户最新 theme-assets.md v2，为 PaneBoard / 文件看板 0.9.x 制作的非官方定制主题。
擎天柱头像由用户上传。本包不表示获得官方授权，也不作为授权证明。
整体配色：深钴蓝、装甲红、钛银、冰蓝。上一版简化机器人已完全移除。

素材制作来源
source/optimus-prime-generated.jpg 为用户提供的 1024×1024 生图头像原件。
应用标识从该图片去除外部白底并提取头盔与面甲，裁去画面边缘不完整的肩部，保留完整头顶构件。该头部被用于状态插画、横幅和侧栏。
source/mechanical-file-atlas.jpg 为上一轮用户提供的 1080×1080 金属文件图标母图。12 种类型图标从母图拆分，深灰边框调整为偏钴蓝，橙色结构点缀调整为装甲红，保留文件类型符号和银色面板。
状态插画使用真实头像与自绘机械界面组合，分别表达空文件夹、搜索无结果和对应文件夹缺失；没有用简化机器人替代头像，也没有声称这些场景是重新生图得到的全身人物场景。
背景由低对比舱壁线条和低透明度头像合成，避免抢占文件列表注意力。
89 个工具栏 SVG 为程序绘制的单色矢量图标。brand.svg 为简化头盔轮廓，仅作可选的单色备用。

文件清单
必需：theme.json；icons/ 下 89 个单色 SVG；app-sizes/app-48.png；types/ 下 12 类各浅深两张，共 24 张 256×256 PNG。
推荐：about.png / about-dark.png；empty-2x.png / empty-dark-2x.png；background.png / background-dark.png。
可选项全部配齐：brand.svg；empty-search.png / empty-search-dark.png；missing.png / missing-dark.png；sidebar-foot.png / sidebar-foot-dark.png。
补充：app.png / app-dark.png；17 档应用图标 PNG；备用 app-48-dark.png；240×160 的 empty.png / empty-dark.png；theme.css；离线预览页；原始母图与校验文件。
按照最新规范，不含 app.ico。程序的任务栏图标固定，换主题不会改变。

尺寸和渲染
app.png：1024×1024，主体最长边约 87.5%，透明底。
app-sizes/app-48.png：48×48，透明底。深色备用文件不假定程序会自动读取。
types/：256×256，主体最长边约 80%；深色版增加浅蓝灰外轮廓，增强深底辨识。
about：640×120，纯色界面底与头像。
empty-2x、empty-search、missing：480×320，外围透明。
sidebar-foot：240×120，外围透明。
background：1920×1080，低对比，均小于 400 KB。
PNG 为 RGBA、非交错、内嵌 sRGB ICC，未新增阴影。
SVG 为 24×24 视图框，描边 1.75、round 端点与连接、currentColor、不固定宽高。单文件小于 8 KB，不含外部字体、滤镜、文本、嵌入图片或外链。

类型映射
folder 文件夹；file 普通文件；file-text 文本/Word/PDF；file-spreadsheet 表格；presentation 演示；file-image 图片；file-video 视频；file-audio 音频；file-archive 压缩包；file-code 代码；app-window 程序；file-cog 设置。
使用规范明确支持的 12 类映射，没有额外假设 Word/PDF/PPT 独立映射已经接入。

使用方式
沿用当前已经可用的主题加载机制，将 optimus-prime 整个目录放入应用的主题目录。MD 未给出系统中的绝对路径，本包不猜测路径，也不自动覆盖应用文件。
theme.json 为顶层 name/author/version/light/dark。浅深两套各包含 11 个配色字段，line 使用 #RRGGBBAA。
彩色素材由程序读取 PNG；工具栏 SVG 由主题着色。不要用彩色 SVG 替换 PNG。
按最新规范，brand.svg 当前不被读取，侧栏使用 app-sizes/app-48.png。
app-dark.png、app-sizes/app-48-dark.png 为备用资源；规范没有确认它们自动被读取。
previews/、source/、theme.css、manifest.json、validation.json 均供审阅或维护，程序不必读取。

预览
previews/index.html 可离线打开，支持深浅切换，提供 18px、48px、96px 的类型图标预览。
previews/theme-overview.jpg 展示头像、类型图标、状态插画、横幅与侧栏。
previews/toolbar.jpg 展示 89 个单色操作图标。

验收与边界
自动检查：命名覆盖、数量、SVG 属性、PNG 尺寸/透明边缘/格式、色板字段、背景体积、文件哈希、ZIP 完整性。
正文和次要文字对背景/卡片/选中/悬停四类底色均达到 4.5:1，语义图标色达到 3:1。
彩色图标的外轮廓增强了深色辨识，未宣称彩色图内每个像素都通过 3:1 对比度检查。
头像使用上传母图实际像素制作；1024px 应用图标包含适量插值。文件类型源图主体约 189 至 231 像素，256px 文件经过小幅插值。尺寸合规不等于新增原生细节。
没有执行真实 PaneBoard 加载、Windows 多 DPI 测试或 18px 用户辨识测试，需以实际应用显示为准。
素材本身无字体依赖；JPEG 预览使用 DejaVu Sans 并已栅格化，HTML 使用系统字体，无网络资源。
