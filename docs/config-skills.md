# Config → Skills 查看

1. 点侧边栏 **Config**，在 **Skills** 中点技能卡片（也可用 Tab 选中后按 Enter／Space）。
2. 在 App 内阅读完整技能正文。顶部显示名称、来源及实际清单路径。
3. **Preview** 显示 Markdown 正文；**Raw source** 显示原始 `SKILL.md`，包括 frontmatter，不改换行或内容。
4. 用 **← Back to Config** 或 Escape 返回列表；返回时恢复卡片焦点。读取失败会显示原因，可点 **Retry**。

这是只读文件查看，不是技能调用：不会执行脚本、启用技能、修改配置、赋予项目信任，也不会把正文插入草稿或发给模型。技能中的指令仅作为文档显示；预览不会自动加载外部图片，本地／相对文件链接也不会借用当前会话目录打开。需要查看图片 URL 或完整元数据时，可切到源码。

## 按需加载与来源

- 列表只展示有界读取的元数据；完整正文仅在点击／重试时读取，上限 **1 MiB**。非 UTF-8、非普通文件、文件消失、过大或来源不符时明确报错，不伪装为成功或静默截断。
- 以实际文件路径区分技能，同名的全局／项目技能不会指向同一份正文。只允许读取当前来源发现清单中的文件，不接受任意文件路径读取。
- 本机项目目录来自会话头部 `cwd`，不从含连字符的编码目录名猜测。保留现有的全局 `skills/*/SKILL.md`、项目 `.agents/skills/*/SKILL.md` 和 `.pi/skills/*/SKILL.md` 清单范围。
- 远程来源只显示已同步的全局技能缓存；不会按远程会话路径读取本机项目。逃出缓存的符号链接不会成为远程技能读取入口。没有缓存的技能不会自动远程下载。
- 数据源切换、返回列表、关闭 Config 后，旧的异步正文／错误不能覆盖新视图。等待来源切换时不可发起读取。

## 清单不是运行时启用状态

Pi 还支持包、设置中的额外资源路径、递归发现、命令行资源及项目信任过滤。这里保留的是查看器的文件清单，不宣称覆盖 Pi 的全部运行时技能，也不推断当前会话是否实际加载／启用了某项技能。

## Verification

`npm run test:config-skills` exercises real components with synthetic data and mocked IPC. Filtered Rust `config_skill_tests` exercise discovery/allowlist/file limits in temporary fixtures. Browser DOM checks can verify interaction, keyboard focus and image suppression with a synthetic IPC bridge. None of these alone is proof of an installed-app click on a private skill, and test records must not publish private skill bodies or configuration secrets.
