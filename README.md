# Pi Desktop (pi-session-viewer)

Pi coding agent 的桌面端(Tauri v2 + React):浏览全部会话、查看对话(含子代理嵌套)、在窗口里直接续聊,并实时显示每个会话**运行在哪**(rmux 分离窗口 / 终端窗口 / 已退出)。

GitHub: https://github.com/roshameow/pi-session-viewer

## 功能

- 📁 **项目分组**:按工作目录列出所有 pi 会话(`~/.pi/agent/sessions/`)
- 💬 **会话浏览**:消息树渲染 — 用户/助手消息、可折叠的 thinking、工具调用卡片、bash 输出、上下文压缩、模型切换、标签;搜索 + 过滤(全部/仅用户/隐藏工具/仅标签)
- 🕸️ **子代理嵌套**:`pi-subagent-durable` 扩展生成的子代理会话自动挂在父会话下(镜像文件 header `id` == 父会话 uuid 精确关联)
- ⏳ **实时续聊**:输入消息 → Rust 直接 spawn `pi --session <file> --mode json`,增量事件流式渲染(text_delta / tool_execution),pi 自动把新消息写回原 JSONL
- 🟢 **运行状态 chip**(每会话):`● rmux` 已附着 / `○ rmux` 分离后台跑 / `✕ rmux` pi 已退出(remain-on-exit 保留窗口)/ `● term` 在终端窗口里跑 — **运行状态与位置解耦**:空闲的 rmux 显示位置 chip 但不显示 running
- ⚡ **rmux 集成**:Attach(附着)、右键 Detach(rmux 内 `Ctrl+G` 或关标签页)、Open TUI(在现有 Terminal 窗口开**标签页**,而不是新窗口)
- ⚙️ **Config 面板**:MCP 服务器 / Agents(全局 + 项目级 `.pi/agents`)/ Skills(全局 + 项目级)
- 📤 导出 HTML、右键删除会话、搜索、可拖拽侧边栏、toast

## rmux 检测原理(为什么可靠)

pi 会清理自己的 argv(只剩 `pi`),`ps -o command=` 永远看不到 `--session`。所以定位靠:
- **每 pi 一个独立 rmux 会话**:`pi-<编码cwd>-<id12>`(uuid 前 12 位,含第 8 位破折号,避免 id8 前缀碰撞),窗口固定 `main`。attach 一个 pi 只影响它自己的会话,tmux 的 attach 不会让其他 pi 的窗口跟着跳
- **@pi_session 窗口选项(权威归属)**:每个 pi 在 `session_start` 时用 `getSessionFile()` 把自己注册进所在窗口的 `@pi_session` 选项(扩展自注册);desktop 建窗口时也写入。map 读选项即可精确归属,不依赖启发式
- **子代理**:在独立 `pi-agents` 会话里,每个子代理一个窗口 `<agent>-task-<taskId>`;map 归到镜像路径,list_sessions 去重时把 rmux 状态合并给真实会话
- **终端 pi**:`comm=pi` 且 tty 不属于任何 rmux pane(`#{pane_tty}` 排除)→ 映射到项目内最新非 rmux 主会话
- **死窗口**:`remain-on-exit` 保留崩溃画面;map 中活窗口优先于死窗口;刷新时自动清理死亡超过 6 小时的死窗口

## macOS 标签页(Open TUI)

Open TUI 通过 bundle 内的 `tab-open-helper` 发送 `Cmd+T`,在现有 Terminal 窗口**开标签页**(标签页是 Terminal 菜单快捷键,唯一不需要改 Terminal 的建标签方式),然后命令跑在新标签里 — 避免把命令打进 raw-mode pi 的 stdin。

**一次性授权(必须)**:macOS 按二进制 hash 记录辅助功能授权,主 app 每次重建都会失效。`tab-open-helper` **从不重建**,授权一次永久有效:

1. 系统设置 → 隐私与安全性 → **辅助功能**
2. `+` → `Cmd+Shift+G` → 输入路径:
   `/Applications/pi-session-viewer.app/Contents/Resources/resources/tab-open-helper`
3. 打开开关,重启 app

> 没授权时回退为开新窗口,并在界面提示原因(如 `osascript is not allowed to send keystrokes (1002)`)。

## 代码签名

本地构建的 adhoc 签名 app 每次重建二进制 hash 都变,会反复丢失 TCC 授权。仓库配置了**稳定自签名证书** `Pi Session Viewer Dev Signing`(tauri.conf.json → `bundle.macOS.signingIdentity`)。新环境需要先在登录钥匙串建好该身份:

```bash
# 一次性创建自签名 code-signing 证书并导入登录钥匙串
openssl req -x509 -newkey rsa:2048 -keyout /tmp/psv.key -out /tmp/psv.pem -days 3650 -nodes \
  -config <(printf '[req]\ndistinguished_name=dn\nx509_extensions=v3\nprompt=no\n[dn]\nCN=Pi Session Viewer Dev Signing\n[v3]\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=codeSigning\nsubjectKeyIdentifier=hash\n')
openssl pkcs12 -export -out /tmp/psv.p12 -inkey /tmp/psv.key -in /tmp/psv.pem -passout pass:psv -legacy
security import /tmp/psv.p12 -k ~/Library/Keychains/login.keychain-db -P psv -T /usr/bin/codesign
```

## 架构

```
Tauri v2 + React 18 + Vite (TypeScript)
├── src-tauri/src/sessions.rs   纯 Rust 解析 JSONL(serde 映射 session v3 格式)
│                               - list_projects / list_sessions / session_detail
│                               - rmux_runtime_map(每 pi 会话 + @pi_session 选项)/ alive_terminal_pis(进程级)
│                               - 多级缓存:(mtime,size) 文件指纹 + 2s TTL 子进程结果
├── src-tauri/src/agent.rs      spawn `pi --mode json`,stdout 逐行转发到 Channel 流
├── src-tauri/src/lib.rs        Tauri 命令注册 + rmux/terminal 集成
├── src-tauri/resources/        tab-open-helper(稳定签名,标签页按键)
└── src/                        React 前端(侧边栏 + 线程渲染 + 输入区)
```

**为什么不需要 Node sidecar**:pi 自带 `--mode json`(增量事件流),且 `pi --session <file>` 续聊会自动把结果写回同一个 JSONL。对话层 = Rust 直接 spawn pi 进程并转发事件,复用本机 pi 配置(扩展、auth、模型)。

## 性能

后端:会话头读取有界(256KB)+ 提前终止;`parse_meta` / `session_detail` / 父调用收集按 (mtime,size) 增量缓存;`rmux list-panes`、`ps`、`lsof` 结果 2s TTL 缓存。前端:线程**窗口化渲染**(默认尾部 150 条 + "show earlier" 按钮),Markdown 块 memo 化。

## 运行

```bash
npm install
npm run tauri dev          # 开发模式
npm run tauri build        # 打包(需 rustup 工具链:$HOME/.cargo/bin)
```

需要:Node ≥ 18、Rust ≥ 1.88(建议 rustup stable)、本机装有 `pi`(从 PATH 或 /opt/homebrew/bin 解析)、`rmux`。

## 测试

```bash
cd src-tauri && cargo test   # 解析层:ISO 时间、真实会话列表/详情、子代理关联率(需本机有会话数据)
```
