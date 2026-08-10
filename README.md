# Pi Desktop (pi-session-viewer)

Pi coding agent 的桌面端:浏览全部会话,点进去查看完整对话,**子代理会话精确嵌套在父会话下**,并且可以直接在窗口里**继续对话**(续聊)。

## 功能

- 📁 **项目分组**:按工作目录列出所有 pi 会话(`~/.pi/agent/sessions/`)
- 💬 **会话浏览**:消息树渲染 — 用户/助手消息、可折叠的 thinking、工具调用卡片、bash 输出、上下文压缩、模型切换、标签
- 🕸️ **子代理嵌套**:`pi-subagent-durable` 扩展生成的子代理会话自动挂在父会话下面(靠镜像文件 header `id` == 父会话 uuid 精确关联,26/27 可关联,无需依赖 agent-logs)
- ⏳ **实时续聊**:输入消息 → Rust 直接 spawn `pi --session <file> --mode json`,增量事件流式渲染(text_delta / tool_execution),pi 自动把新消息写回原 JSONL
- ⏹ 中止当前回合 / 运行中会话绿点脉冲标识 / 窗口聚焦自动刷新

## 架构

```
Tauri v2 + React 18 + Vite (TypeScript)
├── src-tauri/src/sessions.rs   纯 Rust 解析 JSONL(无 pi 依赖,serde 映射 session v3 格式)
│                               - list_projects / list_sessions / session_detail
│                               - 子代理关联:镜像 header.id == 父会话 uuid
├── src-tauri/src/agent.rs      spawn `pi --mode json`,stdout 逐行转发到 Channel 流
├── src-tauri/src/lib.rs        Tauri 命令注册
└── src/                        React 前端(侧边栏 + 线程渲染 + 输入区)
```

**为什么不需要 Node sidecar**:pi 自带 `--mode json`(增量事件流),且 `pi --session <file>` 续聊会自动把结果写回同一个 JSONL。所以对话层 = Rust 直接 spawn pi 进程并转发事件,复用了你本机的 pi 配置(扩展、auth、模型),零额外依赖。

## 运行

```bash
npm install
npm run tauri dev          # 开发模式
npm run tauri build        # 打包 (dmg/app 等)
```

需要:Node ≥ 18、Rust ≥ 1.88(建议 rustup stable)、本机装有 `pi`(从 PATH 或 /opt/homebrew/bin 解析)。

## 测试

```bash
cd src-tauri && cargo test   # 解析层:ISO 时间、真实会话列表/详情、子代理关联率
```

## 已知边界

- 单会话单任务:同一会话同时只跑一个 pi 回合(队列/多并发后续可加)
- 会话树的分支:当前展示活跃分支(parentId 链);`/tree` 式分支切换后续可加
- 图片消息:识别 mimeType 但未渲染图像本体

## License

MIT
