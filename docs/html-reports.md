# HTML 报告：默认浏览器与图片

## 打开方式

macOS 上点击会话中的本地 `.html`／`.htm` 链接时，查看器查询系统为 HTTPS 配置的默认浏览器，然后让该浏览器打开原始文件。默认浏览器是 Safari 就使用 Safari；改成其他浏览器后跟随系统设置。它不使用文本编辑器的 `open -t`，也不依赖 `.html` 文件扩展名是否被关联到代码编辑器。

只查询处理程序配置，不会请求用于查询的 HTTPS 地址；不会修改默认浏览器、启动本地文件服务器、把 HTML 复制成临时页面、注入内容或关闭浏览器安全设置。查询失败会显示错误，不偷偷回退到硬编码的 Safari 或文本编辑器。

源码文件如 `.js`、`.py`、`.sh` 仍按文本查看，不因 HTML 修复而执行脚本。HTML 页面内的 JavaScript 则由外部浏览器按其正常网页安全规则运行，这是 HTML 报告渲染的一部分，不在查看器 WebView 中运行，也没有 Tauri IPC 能力。

## 为什么文本编辑器里没有图

HTML 报告的图不一定是 `<img>`：很多报告用内联 JavaScript 生成 SVG 或 Canvas。文本编辑器不执行网页脚本，即使 HTML 路径正确也不会画出这些图。切换为浏览器是此类问题的第一步。

## 在浏览器里仍然缺图时

- **相对资源**：`<img src="assets/chart.png">` 以打开的 HTML 地址所在目录为基准。查看器保留原文件位置，不把报告移到另一目录或变成 `data:` URL。源目录原有的图片／样式文件也必须存在。文件本身为符号链接时，打开实际 HTML 及其实际资源目录；普通文件不会被重定位到临时目录。
- **根路径**：`src="/assets/chart.png"` 是文件系统／网站根路径，不等于 HTML 旁的 `assets/chart.png`。为直接打开的本地报告，应由报告生成器输出相对路径，或内嵌必要资源。
- **只复制了 HTML**：数据目录面板只复制你明确选择的单个文件，不会自动收集旁边的图片、CSS 或 JSON。副本可能缺少依赖；改为打开完整的原报告，或明确制作含资源的离线报告。
- **动态数据**：页面用 `fetch()`、模块脚本等读取文件时，浏览器的 `file:` 来源限制可能阻止加载。这不等于普通图片路径丢失。不要关闭 Safari 安全设置来掩盖问题；报告生成器可提供自包含报告或你明确启动的受限开发服务。
- **权限／资源本身**：缺失文件、其他 App 的受保护目录、坏链接和浏览器自己的限制仍可能导致失败。查看器不会自动扩大权限或扫描整个项目去替换资源。

如仍有问题，请提供一条具体 HTML 链接及一张图对应的 `src`／浏览器错误（先去掉账号、令牌等敏感信息），再判断是路径、资源缺失还是脚本问题。

## Verification limits / 验证边界

English: On macOS, local HTML reports use the system HTTPS default browser, with the original HTML location retained. No server, temporary page, HTML rewriting or browser-security changes are performed. Source scripts still use a text editor. Missing charts can be non-executed JavaScript rather than missing images; relative assets, root paths, incomplete copies and file-origin restrictions must be diagnosed separately.

离线回归模拟浏览器配置并检查命令参数、原位置和合成资源引用；不能据此保证每份用户报告都能正确渲染。默认浏览器原生查询、外部浏览器渲染和实际报告资源的端到端结果应分别验证。
