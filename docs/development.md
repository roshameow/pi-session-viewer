# Pi Desktop (pi-session-viewer)

Pi coding agent 的桌面端(Tauri v2 + React):浏览全部会话、查看对话(含子代理嵌套)、在窗口里直接续聊,并实时显示每个会话**运行在哪**(rmux 分离窗口 / 终端窗口 / 已退出)。

GitHub: https://github.com/roshameow/pi-session-viewer

## 功能

- 📁 **项目分组**:按工作目录列出所有 pi 会话(`~/.pi/agent/sessions/`)
- 💬 **会话浏览**:消息树渲染 — 用户/助手消息、可折叠的 thinking、工具调用卡片、bash 输出、上下文压缩、模型切换、标签;搜索 + 过滤(全部/仅用户/隐藏工具/仅标签)
- 🕸️ **子代理嵌套**:`pi-subagent-durable` 的显式父 UUID 关联到去重后的主会话或 worker 会话，支持 worker → grandchild（镜像与真实文件的 header `id` 是同一子会话 UUID，不是父 UUID）。缺失、自指、循环或歧义关系不猜测父节点；侧栏主会话分组仍只显示直接子代理，嵌套父标签在子代理区显示
- ⏳ **实时续聊**:输入消息 → Rust 直接 spawn `pi --session <file> --mode json`,增量事件流式渲染(text_delta / tool_execution),pi 自动把新消息写回原 JSONL
- 🟢 **运行状态 chip**(每会话):`● rmux` 已附着 / `○ rmux` 分离 / `ended rmux` worker 已结束但 shell 留存 / `? rmux` Pi 身份未知 / `✕ rmux` pane 已退出 / `● term` 在终端窗口里跑 — **运行状态与位置解耦**:空闲的 rmux 显示位置 chip 但不显示 running
- ⚡ **rmux 集成**:Attach(附着)、右键 Detach(rmux 内 `Ctrl+G` 或关标签页)、Open TUI(在现有 Terminal 窗口开**标签页**,而不是新窗口)
- ⚙️ **Config 面板**:MCP 服务器 / Agents(全局 + 项目级 `.pi/agents`)/ Skills(全局 + 项目级)
- 📤 导出 HTML、右键删除会话、搜索、可拖拽侧边栏、toast

## rmux 检测原理(为什么可靠)

pi 会清理自己的 argv(只剩 `pi`),`ps -o command=` 永远看不到 `--session`。所以定位靠:
- **每 pi 一个独立 rmux 会话**:`pi-<编码cwd>-<id12>`(uuid 前 12 位,含第 8 位破折号,避免 id8 前缀碰撞),窗口固定 `main`。attach 一个 pi 只影响它自己的会话,tmux 的 attach 不会让其他 pi 的窗口跟着跳
- **@pi_session 窗口选项(历史归属)**:每个 pi 在 `session_start` 时用 `getSessionFile()` 把自己注册进所在窗口的 `@pi_session` 选项(扩展自注册);desktop 建窗口时也写入。map 读选项保留位置；只有通过存活/PID 复用校验的独立 runtime slot 才是更强的当前归属证据。选项本身不能证明 worker 进程仍活着
- **子代理**:在独立 `pi-agents` 会话里,每个子代理一个窗口 `<agent>-task-<taskId>`;map 归到镜像路径,list_sessions 去重时把 rmux 状态合并给真实会话
- **终端 pi**:`comm=pi` 且 tty 不属于任何 rmux pane(`#{pane_tty}` 排除)→ 映射到项目内最新非 rmux 主会话
- **死窗口**:`remain-on-exit` 保留崩溃画面;map 中活窗口优先于死窗口;浏览刷新不自动删除窗口或 runtime 文件；清理由 durable owner 或用户显式操作负责

## macOS 标签页(Open TUI)

Open TUI 通过 bundle 内的 `tab-open-helper` 发送 `Cmd+T`,在现有 Terminal 窗口**开标签页**(标签页是 Terminal 菜单快捷键,唯一不需要改 Terminal 的建标签方式),然后命令跑在新标签里 — 避免把命令打进 raw-mode pi 的 stdin。

**一次性授权(必须)**:macOS 按二进制 hash 记录辅助功能授权,主 app 每次重建都会失效。`tab-open-helper` **从不重建**,授权可在辅助程序二进制不变时复用；系统更新或设置变更仍可能要求重新授权:

1. 系统设置 → 隐私与安全性 → **辅助功能**
2. `+` → `Cmd+Shift+G` → 输入路径:
   `/Applications/pi-session-viewer.app/Contents/Resources/resources/tab-open-helper`
3. 打开开关,重启 app

> 没授权时回退为开新窗口,并在界面提示原因(如 `osascript is not allowed to send keystrokes (1002)`)。

## 代码签名

本地 ad-hoc 签名 app 的二进制 hash 随重建变化，TCC 授权可能需要重新确认。仓库保留维护者的既有签名身份 `Pi Session Viewer Dev Signing`（tauri.conf.json → `bundle.macOS.signingIdentity`），但不包含证书或私钥，也不要求新环境创建该身份。

```bash
# 已有该 identity：保留默认签名配置，只打 .app，不生成 DMG
npm run tauri -- build --bundles app --ci
# 没有该 identity：覆盖签名配置，适合本机验证；不是公证发行包
npm run build:unsigned -- --bundles app --ci
```

构建不安装、不启动应用。不要为构建自动创建证书或修改钥匙串；是否替换已安装、正在运行的应用由用户另行决定。可用 `codesign --verify --deep --strict <bundle.app>` 验证本地签名完整性，但通过不代表 Apple 公证或 Gatekeeper 信任。

本地验证注意：`security find-identity -v -p codesigning` 的有效列表可能不列出既有自签名 identity，不能仅凭这一列表判断身份不存在；普通 `find-identity -p codesigning` 可确认 presence。本次既有身份的正常 app-only 构建与严格签名验证通过，而 unsigned 配置产生的 linker ad-hoc 签名缺少 bundle resource seal，严格验证失败。Unsigned 构建只用于开发，不应冒充签名验收通过的发行包。

## 架构

```
Tauri v2 + React 18 + Vite (TypeScript)
├── src-tauri/src/sessions.rs   纯 Rust 解析 JSONL(serde 映射 session v3 格式)
│                               - list_projects / list_sessions / session_detail
│                               - rmux_runtime_map(每 pi 会话 + @pi_session 选项)/ alive_terminal_pis(进程级)
│                               - 多级缓存:(mtime,size) 文件指纹 + 2s TTL 子进程结果
├── src-tauri/src/agent.rs      spawn `pi --mode json`,stdout 逐行转发到 Channel 流
├── src-tauri/src/lib.rs        命令注册(全部 spawn_blocking 后台线程)+ rmux/terminal 集成
├── src-tauri/resources/        tab-open-helper(稳定签名,标签页按键)
└── src/                        React 前端(侧边栏 + 线程渲染 + 输入区)
```

**为什么不需要 Node sidecar**:pi 自带 `--mode json`(增量事件流),且 `pi --session <file>` 续聊会自动把结果写回同一个 JSONL。对话层 = Rust 直接 spawn pi 进程并转发事件,复用本机 pi 配置(扩展、auth、模型)。

## 历史性能观察（开发者本地环境）

**主线程零阻塞(核心修复)**:Tauri v2 的同步命令直接跑在主线程,而 `list_projects` / `list_sessions` / `session_detail` 在 470+ 会话的项目上各要 6-8s —— 打开 app 即冻结 15s+,10s 轮询又重复占满主线程,表现为"一打开就卡死,任何操作都卡"。现在所有重命令都是 `async fn` + `tauri::async_runtime::spawn_blocking`,跑在 tokio 阻塞线程池,避免这些命令直接阻塞 UI 主线程(同 agy_bridge 的 `asyncio.to_thread` 原则)。

**子进程批量化**:`rmux_runtime_map` 原本每个 pane 单独 `kill -0`、每个会话单独 `rmux list-clients -t`、每个注册条目/裸 pane 各一次全量 `ps` 扫描(实测 5s);改为一次 `ps -p <全部pid>` + 一次 `rmux list-clients -F`(本 rmux 无 `-a` 标志,默认即列出全部客户端)批量获取存活/etime/命令行(~100ms)。

**结果级缓存**:`list_projects` 按(会话目录 + agent-log 最新 mtime)指纹 12s 缓存;`list_sessions` 按目录指纹缓存;`list_running` 复用 `parse_meta` 的 per-file (mtime,size) 缓存 —— 10s 轮询不再每轮重读 ~120MB 会话文件头。

**前端**:轮询结果与上次逐字段比对(identity guard),没变化就保持原数组引用 → memo 化的 Sidebar / Thread 跳过重渲染;运行中会话的 detail 只在文件 mtime/size 变化时才重新拉取;流式事件按 rAF 批处理(每秒最多 60 次渲染,而不是每行事件一次);`SessionDetail` 携带 `size`/`updatedAt` 供前端判断。

实测(quantnight 项目,472 文件):`list_projects` 8.4s→0.6s(缓存命中 2ms)、`list_sessions` 6.5s→1.2s、`list_running` 520ms→44ms;前端 279 会话侧边栏 + 14.5MB 巨型会话渲染,曾观察到约 120fps 滚动；该记录未包含完整硬件、采样方法与版本信息，不作为可复现性能保证。

## 运行

```bash
npm install
npm run tauri dev          # 开发模式
npm run tauri build        # 打包(需 rustup 工具链:$HOME/.cargo/bin)
```

需要:Node ≥ 22、Rust ≥ 1.88（选择已有合适工具链）、本机装有 `pi`（从 PATH 或 /opt/homebrew/bin 解析）；RMUX 功能需另装 `rmux`。隔离测试与构建不需要启动 Pi 或 RMUX。

## 测试

```bash
cd src-tauri && cargo test   # 解析层:ISO 时间、真实会话列表/详情、子代理关联率(需本机有会话数据)
```

## Native MCP 与运行状态兼容（2026-10-04）

- Config 面板是**配置清单，不是连接状态**。读取全局 `mcp.json`、会话 header 的真实 cwd 下 `.pi/mcp.json`，保留旧 adapter 的 `.mcp.json`。项目路径不再从编码目录名反推（路径含 `-` 会失真）。同名 global/project 行分别显示来源文件；Pi 在**可信项目**中按项目条目替换全局条目。
- 显示 native `enabled`、`exposure`（缺省 codemode）、`toolExposure`，并分别显示 adapter `disabled`、`socket`、`directTools`。Adapter 的 disabled/directTools 不是 native 选项；不要把清单展示当成已切换 backend。禁用 builtin:mcp 或由 adapter 注册 `/mcp` 替换内建时，实际行为由 Pi 决定；扩展动态注册的 session-only servers 不在静态文件清单内。远程仅展示已同步的全局配置，不探测本机上的远程 cwd。
- 续聊使用 **Pi CLI JSON 模式，不是 SDK**；显式对齐进程 cwd（已核实当前 Pi CLI 也从 SessionManager header 重建 runtime/resource cwd；进程 cwd 是防御性对齐 bootstrap 与相对 CLI 路径），不限制 tools、不禁用扩展，保留 Pi 内建 MCP/codemode/tool_search 的发现与替换机制。stderr 并行读取并显示诊断，避免 MCP 启动日志堵塞管道。若未来改用 `createAgentSession`，SDK 不自动加载 builtin：应在 `DefaultResourceLoader.extensionFactories` 添加 `createMcpExtension()`、`createCodemodeExtension()`、`createToolSearchExtension()`，reload 后 bindExtensions；以实际 Pi SDK 文档和 replaceable/builtin 语义为准，勿直接混用 CLI 假设。
- `mcp__<server>__<tool>`、`codemode`、`tool_search` 和旧 adapter 工具名均保留原名渲染。实时 nested/parallel 调用按 `toolCallId` 关联 update/end，不再改写“最后一个工具”的结果；展开可看完整参数（包含 script code）。持久化 transcript 不会伪造 nested 子调用条目。
- `rmuxDead` 仅表示 `pane_dead=1`；`rmuxPiAlive` 是 true/false/null。独立 PID runtime slot / 实际 Pi task argv 是存活证据；shell、tee、tail 或 `read` wrapper 中提到 task 路径不算 worker alive。已结束 worker 的留存 shell 显示 `ended rmux`；缺进程快照或无可验证 SDK/runtime 身份显示 `? rmux`，而非 dead/running。正常 idle TUI 和仍有精确存活身份的 settled worker **不会**因 agent_end/agent_settled 被判死。历史 pane 仍可 attach 浏览（Attach 可浏览 dead pane；Open TUI 重建仍走既有用户操作）。

### 隔离回归验证（不启动 Pi / MCP / rmux / SSH / desktop）

```bash
npm run test:adaptation
npm run build
PATH="$HOME/.cargo/bin:$PATH" cargo test --offline --manifest-path src-tauri/Cargo.toml adaptation_tests
```

新增 Unix fake CLI 用 Python 3，输出超过 1MiB stderr；复用真实 command builder 与 stderr drainer，以 5 秒本地超时验证 cwd 与退出，不执行真实 send_message。macOS 临时目录可能以 `/var` 或 `/private/var` 表示，测试用 canonicalize 比较 cwd。

全量旧 Rust 测试包含读取真实本机会话、调用 rmux 的 smoke/dump 测试，**不适用于 mock-only 验收**。选已安装的 Rust ≥1.88 工具链；无需升级依赖。私有检查点、源码审计和构建日志保存在被忽略的 `/artifacts/`，不随源码发布。

## 0.1.2 黄点兼容推断边界

- 黄点（busy）是**已验证 Pi 身份 + transcript 最后一条有效消息仍 pending** 的兼容推断（user、assistant `toolUse` / toolCall、toolResult），**不是 SDK 实时 `isStreaming`**。已验证 Pi 的长思考或工具执行，不会仅因超过 60 秒未写 JSONL 而失去黄点；最后 assistant 为 stop / error / abort 等完成状态时，不点亮 idle TUI。
- UNKNOWN 身份只允许短暂 fresh transcript 的弱兜底；明确已结束 Pi / 留存 dead shell 仍为 false，不把终端存活当 busy。
- 识别 anchored `pi-subagent-task-*` 进程标题，不匹配 shell / tee 内的文本；正常 `pi_subagent_exit(exitCode=0)` 元信息不掩盖此前 `agent_settled`。
- 远程 PID age guard 使用采集时的源主机时间；旧快照使用固定 snapshot mtime。同步过滤包含 `pi-subagent` 标题，但远程仍是**最近一次手动同步的快照，不是实时状态**。

兼容推断无法全面保证 auto-retry、compaction、超大 / 截断 JSON 或 SSH 降级场景；纯回归和一次本机只读 inventory 不等于 GUI / 远程端到端验收。

隔离检查另加 `cargo test --offline --manifest-path src-tauri/Cargo.toml running_diagnostics -- --skip live_running_inventory`（5 个纯回归）。`live_running_inventory` 默认 ignored；经授权可用 `PI_VIEWER_DIAGNOSTIC_PROJECT=<编码项目目录名>` 只读核验该项目，不调用全项目 `list_projects` / `list_running`，不启动或控制 Pi / RMUX。

## 0.1.6 bounded detail transport

Normal detail calls are `session_detail_page` and explicit `session_entry_body`, not a full-session IPC result. Initial index is readonly and retains offsets/relationships rather than bodies. Source/canonical path and inode/ctime/nanosecond-mtime/size generations guard reads and cursors. Index/parsed caches use estimated byte budgets; these estimates are not process RSS/physical footprints. Default 100 entries and 256 KiB apply to the entire serialized page; giant entries expose a lossless original-record chunk action. Counts/search/filter cover the selected branch, not only downloaded rows; raw JSONL download preserves fields the UI renderer does not interpret.

Legacy explicit full detail/export stays available but has no unbounded retained deep-clone cache. Header-only getters avoid parsing content for CWD-dependent terminal/attach/send actions. Existing Pi CLI/model/task gates and authentication are unchanged.

Run small isolated checks serially: `node scripts/test-bounded-detail-app.mjs`, `node scripts/test-bounded-detail-thread.mjs`, and `CARGO_BUILD_JOBS=1 PATH="$HOME/.cargo/bin:$PATH" cargo test --offline --manifest-path src-tauri/Cargo.toml sessions::detail -- --test-threads=1`. Frontend 52 MiB stress is opt-in with `PI_VIEWER_FULL_BODY_STRESS=1`; real-file aggregate benchmark is ignored unless explicitly approved. Do not run heavy tests concurrently on a pressured desktop.
