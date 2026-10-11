# 记住数据目录授权 / Remembered data directories

任务需要微信附件等其他 App 的文件时，不应仅凭 macOS 弹窗里的 `pi-session-viewer` 名称判断是否允许。任务的 Python、浏览器等子进程也可能被 macOS 归到查看器名下。

## 推荐工作流

1. 点击侧边栏 Config 旁的文件夹锁小图标（鼠标悬停提示 **Data access / 数据目录**），选择 **选择目录并记住授权**。
2. 在系统文件选择器中选择实际需要的子目录，例如微信某个账号的附件目录。不要选择整个用户目录、`Library`、`Containers` 或 `Group Containers`。
3. 查看器保存 macOS 的目录授权凭据（security-scoped bookmark）。重复使用这个读取入口时尝试复用凭据；目录移动、授权失效或系统不支持恢复安全作用域等情况下需要重新选择。当前查看器未启用 App Sandbox，跨重启读取受保护微信目录是否免提示需要在安装的 App 中验证，不保证永久授权。
4. 点击浏览后，才列出该目录的内容。选择需要的单个文件复制到查看器自己的私有存储，原目录不被写入。大目录分页面显示，不递归扫描；单文件上限为 100 MiB。
5. 如需交给任务，明确点击 **插入当前会话草稿**，检查后再发送。任务应处理副本，不再直接访问微信原目录。

授权或复制本身不会自动发送文件内容到模型。**发送草稿并让任务读取副本后，文件内容可能被交给配置的模型／工具／服务。** 私密聊天、账号信息和附件请先审查。

## “记住授权”的边界

- 这是查看器自己的数据读取入口，不是任意 Python、Pi、MCP、RMUX 或终端任务的文件系统沙箱。
- macOS 的泛化“其他 App 数据”弹窗不是一个可由查看器配置的永久路径白名单。仅保存普通路径不能替代系统授权。
- 通过系统文件选择器取得的目录权限和 bookmark 需要在查看器进程中正确使用。直接访问原目录的任务脚本仍可能触发系统提示；本功能不会自动改写脚本，也不会静默扩大权限。
- 首次仍需要你在系统选择器中明确选择。书签失效、目录不可用或系统拒绝时显示错误，不会自动要求“完全磁盘访问”，也不会切换成未经授权的普通路径读取。
- **移除授权**停止此读取入口的后续访问，删除查看器保存的凭据；不承诺撤销 macOS 已授予的所有权限。
- **文件副本单独保存**。移除源目录授权不会删除副本，请使用副本列表里的删除功能；已经传给任务／模型或被任务另存的内容也不能撤回。
- 数据目录与副本属于本机，不自动同步到远程会话；远程会话不能插入这些本机路径。

## 安全设计

目录浏览和复制接口只接受已授权目录的 ID 和相对路径；禁止绝对路径、`..`、子路径符号链接和非普通文件。目录页不是文件系统快照：源目录增删文件时，应重新读取，页间可能发生变化。访问源文件只读，文件副本使用独立名称和私有权限，不覆盖现有文件。书签不返回前端或模型。

这些限制针对读取入口；不限制同一用户下其他进程的既有权限。

## 本地存储与清理

授权书签和导入记录位于 Tauri 的应用数据目录下，不在项目仓库中。macOS 默认是 `~/Library/Application Support/com.pidesktop.viewer/`：`data-access/manifest.json` 保存私有记录，`imports/<随机 ID>/<文件名>` 保存明确选择的副本。目录和文件使用当前用户的私有权限；不要把书签、导入记录或副本提交到 Git、上传或公开分享。

优先使用面板的“撤销目录授权”和“删除副本”，避免手动移动文件后记录与路径不同步。副本已经被其他工具移除或删除中途失败时，可以重试“删除副本”清理记录；只删除所管理的文件，不递归清除同目录中由你或任务生成的其他文件。书签恢复会核对所选目录身份；目录移动、被替换或授权无法恢复时要求重新选择，不会改读同名的其他目录。当前进程中由系统选择器取得的目录访问保持到撤销或退出查看器；这不等于所有子进程的持久权限。

## Verification limits / 验证状态

English: This is a viewer-owned, read-only, explicit import workflow, not an OS sandbox or permanent allowlist for arbitrary tasks. The folder-lock icon beside Config opens the native chooser. Saved credentials are reused only when the system can restore authority; failures require reselection instead of ordinary-path retries. Copying and insertion into an idle local draft are separate actions, never automatic uploads. Revoking a source does not delete copies or retract data already sent to a model. Cross-restart protected-container access in the current nonsandboxed build remains unverified.


源码构建和离线测试不等于微信目录端到端验收。需要在安装的 macOS App 中手动验证：首次选择、取消选择、重启后恢复、浏览／复制、移除授权、删除副本，以及授权失效提示。验证前不保证“永久无弹窗”。测试时使用你明确愿意授权的测试文件夹，不自动读取真实微信内容。

参考：[Apple WWDC23 — What's new in privacy](https://developer.apple.com/videos/play/wwdc2023/10053/)（App 数据容器、NSOpenPanel 与系统授权的说明）。
