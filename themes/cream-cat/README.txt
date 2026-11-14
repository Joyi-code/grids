奶油猫咪 · 主题素材包 v2.0.0

交付目标
按用户上传的主题素材规范 v2，为 PaneBoard / 文件看板 0.9.x 提供完整主题包。

素材来源与处理
彩色图标及猫咪形象来自用户上传的生图模型母图，未用程序重画替代。
实际收到的附件为 1080×1080 JPEG，原始副本保存在 source/generated-atlas.jpg。
母图按四行四列拆分，去除外部白底，保留物体内部奶油色，修正白底边缘后导出透明 PNG。
12 种文件类型来自前三行，应用图标来自第四行第一格，其余三格分别用于空文件夹、搜索无结果、对应文件夹不存在。
深色版本采用轻微降亮度和浅奶油外轮廓处理，保留原图内部线条与表情；没有再次请求生图生成另一套角色。
横幅、背景、侧栏插画由同一套已提取的生图元素排版合成。
89 个工具栏 SVG 与 brand.svg 为程序绘制的单色几何线条，以满足小尺寸和 currentColor 渲染规范，不是从生图母图提取。

文件说明
根目录 theme.json：name/author/version 与 light/dark，两套色板均含规范指定的 11 个字段。
icons/：89 个独立单色 SVG，24×24，描边 1.75，圆端点、圆连接。
types/：12 类 × 浅深两套，共 24 张 256×256 PNG。主体最长边约占 80%，透明底。
app.png：1024×1024 应用图标，透明底。
app.ico：16、24、32、48、64、128、256 共七档。
app-sizes/：包含必需的 app-48.png，以及 16 至 1024 的补充尺寸。
about.png / about-dark.png：640×120，纯色底横幅。
background.png / background-dark.png：1920×1080，角落低对比猫咪装饰，单张小于 400 KB。
empty-2x.png / empty-dark-2x.png：480×320 空文件夹插画。
empty.png / empty-dark.png：240×160 补充版本。
empty-search.png / empty-search-dark.png：480×320 搜索无结果插画。
missing.png / missing-dark.png：480×320 对应文件夹不存在插画。
sidebar-foot.png / sidebar-foot-dark.png：240×120 侧栏插画。
brand.svg：24×24 单色侧栏标识补充版。
theme.css：同一套色板的 CSS 变量辅助文件，应用不必读取。
previews/index.html：离线可交互审阅页，点击按钮切换深浅模式，含 18px、48px、96px 实际图标尺寸预览。
previews/generated-assets.jpg：彩色素材总览。
previews/toolbar.jpg：89 个操作图标总览。
validation.json：自动检查结果及未验证事项。
manifest.json：每个文件的 SHA-256 和体积，不包含该清单自身。

文件类型名称
文件夹 folder；普通文件 file；文本/Word/PDF file-text；表格 file-spreadsheet；演示 presentation；图片 file-image；视频 file-video；音频 file-audio；压缩包 file-archive；代码 file-code；程序 app-window；设置 file-cog。
本包使用规范明确支持的 12 类映射，不额外虚构尚未接入的 Word/PDF/PPT 独立类型。

接入方式
将 cream-cat 整个目录放入应用约定的主题存放位置，或按既有主题加载方式替换对应素材。文档未提供主题目录的系统绝对路径，本包不假设该路径，也不附自动修改程序的脚本。
彩色素材务必使用 PNG；工具栏 SVG 由程序按主题染色。
按 v2 文档，background*.png、app.ico、brand.svg 的显示仍取决于代码接入状态：背景接入中，任务栏 ICO 需代码替换，当前侧栏使用 app-sizes/app-48.png。
原文未确认 empty-search、missing、sidebar-foot 的深色后缀是否已有加载逻辑，本包提供配对文件，必要时由开发者加入读取。

像素与质量边界
上传母图为 1080×1080，单个图标实际主体约 160 至 236 像素；256px 文件类型图标和 1024px 应用图标均从该母图插值导出。尺寸符合规范，但不代表原生 1024px 细节。
保留生图中的纹理和笔触，未伪造矢量母版。特别是超大应用图标不具备原生矢量锐度。
深色 PNG 的浅色外轮廓增强边缘辨识；色板和轮廓色做了数值对比度检查，但未宣称彩色图内每个像素都达到 3:1。
提供 18px 预览供人工确认，未声称已完成用户辨识测试或真实应用验收。

技术与验收
PNG 为 RGBA、透明或规范允许的纯色背景，内嵌 sRGB ICC、非交错、无投影。
SVG 不含外部字体、滤镜、文字、图片、脚本或外链，单文件小于 8 KB。
图标素材无字体依赖。JPEG 预览使用 DejaVu Sans，文字已栅格化；HTML 仅使用系统字体，无在线资源。
已检查文件名覆盖、PNG 尺寸、透明边缘、ICO 七档、SVG 属性、色板字段、背景大小、文件哈希及 ZIP 解压完整性。
未执行真实 PaneBoard 加载、Windows 多 DPI、18px 人工可辨识性验收。
