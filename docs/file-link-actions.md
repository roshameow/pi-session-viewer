# 本地文件链接：打开与 Finder 定位

## Markdown 使用自己的关联应用

macOS 上的 `.md`、`.markdown`、`.mdx` 是文档，不应被统一的 `open -t` 强制交给默认纯文本编辑器。普通非可执行 Markdown 按系统文件关联打开：如果你配置的是 VS Code／VS Code Insiders，就由对应编辑器打开，而不是硬编码某一个 Code 安装路径。

这不会修改系统文件关联。HTML 仍使用默认浏览器；Python、JavaScript、Shell 等脚本仍强制作为文本查看，不能借文档别名执行程序。带执行权限或实际指向其他类型的文件仍须通过安全检查，不能仅凭链接显示为 `.md` 就放行。

## 右键使用查看器自己的原生动作

在本地文件链接上右键，查看器显示紧凑菜单：

- **打开文件**：与左键使用同一条安全打开流程。
- **在 Finder 中显示**：按当前会话项目目录解析链接后，在 Finder 中定位文件，不尝试在 WebView 中导航到那个地址，也不启动文件本身。

右键本身不会打开、读取内容或定位文件；必须再选择动作。桌面 HTTP(S) 链接另有“在默认浏览器中打开”菜单，回环网页需验证映射后才可定位文件；不能把网页地址直接当路径，见[网页链接说明](web-links.md)。纯浏览器预览保留普通网页菜单。菜单可用键盘打开／导航，Escape 关闭并返回链接；选择动作、点击外部、滚动或来源改变时关闭。

Finder 动作只在 macOS 支持。远程来源、浏览器演示、未就绪会话及过期的会话／来源操作不能打开或定位本机同名文件。链接与会话身份在调用前重新检查；失败在原链接旁提示，不自动重试或请求更大的权限。

定位使用实际目标：符号链接指向的文件会定位到实际文件。Finder 定位不是执行操作，所以与“打开文件”的文件类型限制不同；不能把 `-R` 悄悄替换成普通打开来兼容不支持的平台。

## Verification limits / 验证边界

English: Non-executable Markdown follows its macOS file association (including VS Code/Insiders), rather than the plain-text editor. HTML keeps default-browser routing, scripts retain forced-text safety. The viewer's explicit local-link context menu routes Reveal in Finder through native session-aware path resolution; right-click alone performs no filesystem action. Source/epoch guards and visible errors apply to both actions; remote/preview paths never resolve as local files.

合成文件、真实 Markdown 渲染器和模拟 IPC／命令参数测试不等于所有用户文件的原生打开验收。系统关联查询、Finder 进程启动与实际文件高亮属于不同验证层；不据此保证所有自定义编辑器或所有平台已经端到端通过。
