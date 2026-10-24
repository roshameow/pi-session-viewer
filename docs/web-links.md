# 网页链接与 localhost 开发预览

会话里的“查看模型／查看报告”链接可能是 `http://127.0.0.1:5173/examples/model/viewer/`，并不是一个本地 `.html` 文件。两者不能用同一条文件路径逻辑处理。

## 在默认浏览器中打开

桌面 App 中的 HTTP／HTTPS 链接通过显式点击交给系统默认浏览器，而不是只依赖 WebView 的 `target="_blank"`。保留原始协议、主机、端口、路径、尾部斜杠、查询和片段；不会把开发预览转换为 `file://`，否则 Vite 模块、模型及相对资源可能无法加载。

- 普通网页打开不读取会话项目目录，也不要求来源是 Local；远程会话中的网页链接也可打开。
- `localhost`／`127.0.0.1` 始终指当前电脑。查看器不自动 SSH、创建端口转发、启动／重启开发服务器。
- 纯浏览器演示保留普通网页链接行为，不调用桌面 API。
- 不支持的协议、带凭据或无效的 URL 不会交给 shell 或当作文件执行。打开失败显示在链接旁，不再无声失败。

## localhost 的 Finder 定位

普通互联网网址没有可直接交给 Finder 的本地路径。桌面右键菜单首先提供 **在默认浏览器中打开**；可识别的回环地址另外提供 **在 Finder 中显示对应文件**。

Finder 定位必须满足：

1. 当前是已就绪的本机会话，使用会话头部记录的真实项目目录，不从编码会话目录名猜路径。
2. URL 路径对应项目内的普通候选文件；目录 URL 检查 `index.html`。拒绝路径穿越和逃出项目的符号链接。
3. 显式选择动作后，才对回环服务做有界请求，核对响应与候选文件一致。HTML 只容忍已知 Vite 客户端注入、严格受限的模块时间戳缓存参数及空白行差异，不随意删除脚本或重写内容。
4. 通过核对后才原生定位文件。无法确认对应关系时显示错误，不把网址当成本地路径，也不随便打开项目里的同名文件。

该定位流程不适用于所有服务器路由：动态页面、不同根目录、重定向、带查询参数的 URL、HTTPS 回环映射、超过校验上限的内容或不匹配的响应会明确失败。当前有界响应上限为 1 MiB，请求设置连接与总超时；不跟随重定向、不使用代理或凭据，不放松 TLS／浏览器安全设置。普通外部网页没有 Finder 定位选项；远程会话不能把 URL 映射成本机文件。

## 区分三类故障

- **App 链接分发失败**：服务器及页面本身正常，但 WebView 没把 HTTP 链接交给外部浏览器。
- **服务器／资源失败**：端口未监听、返回 404、模型／图片不存在。改浏览器并不能修复这些问题。
- **页面渲染失败**：HTTP 200 不代表模型已经显示；还要检查模块、模型响应、页面错误、画布和实际画面。不得把空白页或替代模型当成已修好。

## Verification limits / 验证边界

English: Desktop HTTP(S) clicks go to the system browser with the original URL intact, including localhost ports and trailing slashes. This is separate from local-file opening. Loopback Finder reveal is an explicit, guarded, bounded content-verified mapping inside the selected local session project; uncertain mappings fail instead of guessing. No developer server is started, no URL is rewritten to file, and no remote forwarding or browser security changes occur.

真实 Markdown 解析、模拟 IPC、原生命令参数、有界校验及具体地址诊断是不同层次。隔离浏览器能显示某页面，不等于 Safari 或已安装 App 的点击流程也已经端到端通过；验证记录应分别说明，不公开用户会话、页面或模型截图。
