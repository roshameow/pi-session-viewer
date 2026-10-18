# 会话内本地文件链接

会话中的 `file:///…`、绝对路径、`~/…` 和项目相对文件链接通过明确点击交给系统打开，而不是尝试在 WebView 中加载本地文件。相对路径以所选会话头部记录的项目工作目录为准，不以查看器的启动目录或 JSONL 所在目录为准。

- 支持本地会话中的 Markdown 链接，保留普通网页链接的默认 URL 安全过滤。
- URI 中的空格和 Unicode 转义仅解码一次；`file://localhost` 是本机，其他主机的 `file://` 不作为本地文件打开。字面 `%` 请写成 `%25`，例如文件名 `report%20.md` 的链接写成 `report%2520.md`，避免与 `report .md` 混淆。
- 本地目标文件不存在、系统打开失败或文件类型不支持时，在链接附近显示错误，不自动重试或提升系统权限。
- 远程会话、纯浏览器预览和未就绪来源不能把远端路径当成本机文件打开。不会自动 SSH、下载或打开远程缓存里的同名文件。
- `#L…` 或行列标记不意味着跳转代码行；打开的是文件。详情取决于操作系统和默认编辑器，不保证行号定位。
- macOS 普通非可执行 Markdown 按自己的系统文件关联打开（如 VS Code／Insiders）；脚本／其他源码文本仍按文本查看，避免按脚本自身的关联程序执行。应用包、未知可执行文件及危险类型不会作为程序运行。其他平台的文件打开支持有明确限制，不声称已端到端验证。
- 仅点击时调用文件打开命令。查看器不为渲染链接而扫描目录、读取文件内容或发送文件到模型；用户打开文件仍须遵守 macOS 原有授权。

## 常见链接与平台范围

| 链接示例 | 解析方式 |
| --- | --- |
| `[报告](artifacts/report.pdf)` | 所选会话项目目录中的文件，不是查看器运行目录 |
| `[代码](src/main.ts:12:3)` | 先尊重确实存在的同名文件，否则移除行／列后缀；不保证跳转行号 |
| `[文件](file:///tmp/report%20one.pdf#L12)` | 本机文件 URI，空格解码一次；行号只是定位装饰 |
| `[附件](~/Downloads/report.pdf)` | 当前用户的 HOME 路径，仍需系统允许访问 |

macOS 支持显式识别的文本／源码、HTML、PDF、常见 Office／图片格式和文件夹。`.html`／`.htm` 报告使用系统 HTTPS 默认浏览器打开（Safari 是默认时使用 Safari），保留原始报告位置以解析相对资源；详见 [HTML 与图片说明](html-reports.md)。普通非可执行 Markdown 使用自身系统文件关联（如 VS Code／Insiders）；脚本／其他源码文本使用 `open -t` 默认文本编辑器；普通文档／文件夹使用默认关联程序。右键“在 Finder 中显示”走独立的原生定位操作，见[文件链接操作](file-link-actions.md)。未知／二进制类型、程序、安装器及应用包会被拒绝，符号链接也按实际目标类型检查，不会从被拒绝的目标改开另一份文件。

Linux 使用绝对路径调用 `xdg-open`，拒绝脚本及可执行文件；尚未验证原生桌面效果。Windows 的本地打开命令明确返回不支持，不通过 `cmd /c` 拼接文件路径。网络主机的 `file://`、协议相对路径、危险／未知 URI 协议、控制字符和不支持的文件 URI 查询／片段不作为本地文件打开。

本功能与“记住数据目录授权”的读取入口不同：打开外部默认应用不是给所有 Pi/Python 任务授予该目录的永久权限。

## Verification limits / 验证边界

English: Local Markdown file links are click-only and use the selected local session's CWD. HTML reports use the macOS HTTPS default browser without relocating the file; non-executable Markdown follows its own file association, other source/text files use the text editor, and supported documents/folders use default handlers. Reveal in Finder is an explicit native action, not WebView navigation. Invalid or denied targets produce visible errors. Remote/preview paths are never guessed as local files. These APIs neither upload file contents nor grant arbitrary Pi/Python access. Native handler behavior still needs manual verification.

离线测试使用合成文件、模拟 IPC 和命令参数检查，不启动默认关联应用；实际已安装 App 中的 Finder、编辑器、PDF 阅读器打开效果需要手动验证。
