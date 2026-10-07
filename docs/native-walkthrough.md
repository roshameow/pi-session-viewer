# 原生演示 / Native walkthrough

Pi Desktop 与 Pi TUI 协同：需要深入交互时 **attach**；Agent 自主执行时 **detach**，用 Desktop 侧边栏掌握多项目、会话与子代理总览。

| 资产 / Asset | 中文 | English | 时长 / Duration |
| --- | --- | --- | --- |
| 无声 GIF / Silent GIF | [GIF](assets/native-workflow.zh.gif) | [GIF](assets/native-workflow.en.gif) | 19.04 s |
| 无声 MP4 / Silent MP4 | [MP4](assets/native-workflow.zh.mp4) | [MP4](assets/native-workflow.en.mp4) | 19.04 s |
| PNG 封面 / Cover | [PNG](assets/native-workflow.zh.png) | [PNG](assets/native-workflow.en.png) | 首帧 / First frame |

## 真实性与边界 / Scope

- 原生 macOS UI、Pi TUI、RMUX attach/detach 与模型执行为真实实录；工作区与种子 history 为合成示例，不是私人项目，也不是 SVG 概念动画。
- 短版保留完整 **总览 → attach → TUI 补充指令 → detach → 总览**。既有运行证据确认：同一 pane detach 后仍完成工具写入；本次只重编码既有素材，没有重新调用模型或录制。
- 仅演示本地多项目；不证明实时 remote。远程缓存仍是同步快照，平台与源码构建限制见 README 技术正文。
- 放大、高亮与字幕均为后期处理；末尾是任务完成后的独立总览镜头，不能将其当作任务仍在执行的证明。
- 原素材已有路径遮罩；本次还遮盖了约 11–12.32 秒 detach 转场中的终端标题、路径状态栏和 shell 身份。保留真实 detached 提示与任务内容，不复制 raw、私人路径或原始审阅截图。
- 本仓库仅提供无声 GIF / MP4 与 PNG。约 40 秒的既有 AI 旁白版仅保留在 `launch-ops` 本地 `artifacts/pi-desktop/narrated-demo-{zh,en}-v1/narrated.mp4`；Edge 消费者 TTS 声音的公开／商业许可尚未确认，不随本次 README/git 更新发布。

**English:** Real native UI, Pi TUI, RMUX and model execution, with a synthetic workspace and seed history. The silent short completes the entire loop. AI-narrated versions remain local in `launch-ops` and are excluded from this publication pending confirmation of public/commercial voice licensing. Zooms/highlights and metadata-only privacy masks are editorial. The ending is a separate post-completion overview take. Local projects only, not live remote telemetry. No new capture/model/TTS calls.

## 生成与验收 / Reproduction and checks

输入来自制作仓库 `artifacts/pi-desktop/rendered/native-workflow-v2-{zh,en}/short.mp4`（各 19.04 秒），**不是仅开头 8 秒的 `preview.gif`**。先完成隐私遮罩，再从无声 MP4 生成 GIF 和首帧 PNG。输入哈希、遮罩滤镜、工具版本及全部输出验证见 [verification JSON](assets/native-workflow.verification.json)。

```bash
# INPUT 必须是已脱敏的完整短片；OUTPUT 为新文件。
ffmpeg -n -i "$INPUT" -filter_complex \
  '[0:v]fps=10,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle' \
  -loop 0 -final_delay 14 "$OUTPUT"
ffprobe -v error -count_frames -show_streams -show_format -of json "$OUTPUT"
ffmpeg -v error -i "$OUTPUT" -f null -
```

已验证两种 GIF 均为 **960×540、190 帧、19.04 秒、无限循环**（前 189 帧各 0.10 秒，末帧 0.14 秒），保留最后的 Desktop 总览。两种无声 MP4 均为 476 帧 / 25 fps。6 个媒体文件全部完整解码通过；链接存在、技术正文未变，`git diff --check` 通过。媒体总大小约 2.46 MiB。

**本轮可复用经验：**审阅均匀 contact sheet 不足以发现短暂泄露，必须另外检查 detach 转场逐帧；已有“脱敏”短片也须复核 shell 身份。10 fps 会将 19.04 秒短片量化为 19.00 秒，`-final_delay 14` 可保留末尾展示至 19.04 秒，不需要裁掉闭环或补造实时画面。

本次更新仅涉及双语 README 演示首屏与文档素材；应用源码未改动，也没有重新运行模型或操作用户窗口。
