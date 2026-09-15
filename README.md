# Pi Desktop

[English](README.en.md) · [源码](https://github.com/roshameow/pi-session-viewer) · [开发说明](docs/development.md)

Pi coding agent 的桌面会话工作台：按项目浏览历史对话，查看父子代理关系，定位运行中的终端，并在桌面窗口继续会话。

## 先看演示

使用合成会话浏览真实的侧边栏与对话组件，不需要安装 Pi、RMUX 或配置模型：

```bash
git clone https://github.com/roshameow/pi-session-viewer.git
cd pi-session-viewer
npm ci
npm run demo
```

打开 `/demo.html`，切换主会话和子代理，展开工具调用并试用搜索/过滤。演示仅展示合成记录，不连接本机会话目录，也不调用模型。

## 桌面功能

- 按工作目录组织会话，浏览消息、thinking、工具调用和输出。
- 将 `pi-subagent-durable` 子会话嵌入父会话列表。
- 区分终端位置与运行状态，支持 RMUX attach/detach。
- 通过本机 Pi 继续会话，复用已配置的模型与扩展。
- 浏览 Agents、Skills 和 MCP 配置，导出会话 HTML。

## 安装状态与平台

目前提供源码构建，尚无已验证的正式安装包。不要将本地自签名构建视为已公证的 macOS 发行版。

| 环境 | 当前说明 |
| --- | --- |
| macOS | 主要开发平台；终端标签页助手与辅助功能设置见开发文档 |
| Linux / Windows | 代码含部分平台分支，尚未提供完整桌面功能验证或发行包 |
| 浏览器演示 | 仅合成会话预览，使用 Node.js 22+ |

## 从源码运行

需要 Node.js 22+、Rust 1.88+、[Tauri 系统依赖](https://v2.tauri.app/start/prerequisites/)及本机 Pi。RMUX 用于终端会话管理；需使用该功能时单独安装。

```bash
npm ci
npm run tauri dev
```

macOS 本地打包可运行 `npm run build:unsigned`；它覆盖开发者证书配置，不需要创建同名私有签名身份。用于分发的签名、公证及系统授权仍需自行处理。已有稳定签名配置的维护者可继续使用 `npm run tauri build`。

## 开发与反馈

- [实现、签名和历史性能记录](docs/development.md)
- [版本记录](CHANGELOG.md)
- [关联项目：pi-subagent-durable](https://github.com/roshameow/pi-subagent-durable)
- [提交问题](https://github.com/roshameow/pi-session-viewer/issues)：请提供系统、Pi 版本、复现步骤；会话日志请先脱敏。

[MIT](LICENSE)
