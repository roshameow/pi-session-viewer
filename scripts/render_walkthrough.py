#!/usr/bin/env python3
"""Render a conceptual walkthrough, never a capture of the application.

Requires Pillow and ffmpeg. All content is synthetic; no Pi sessions are read.
The palette follows src/styles.css; examples follow src/demo/fixtures.ts.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import subprocess

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
W, H, FPS, SECONDS = 1280, 720, 12, 20
C = {
    "bg": "#111418", "panel": "#171b21", "raised": "#1e242c",
    "line": "#2a3340", "fg": "#d8dee9", "dim": "#8a94a6",
    "cyan": "#00d7ff", "green": "#b5bd68", "purple": "#b79ae8",
}
FONT_DIR = Path(os.environ.get("PI_DEMO_FONT_DIR", "/usr/share/fonts/truetype/dejavu"))
FONTS: dict[tuple[int, bool, bool], ImageFont.FreeTypeFont] = {}


def font(size: int, bold: bool = False, mono: bool = False):
    key = size, bold, mono
    if key not in FONTS:
        name = "DejaVuSansMono" if mono else "DejaVuSans"
        name += "-Bold.ttf" if bold else ".ttf"
        path = FONT_DIR / name
        if not path.is_file():
            raise SystemExit(f"Missing {path}; set PI_DEMO_FONT_DIR to a DejaVu font directory.")
        FONTS[key] = ImageFont.truetype(str(path), size)
    return FONTS[key]


def frame(t: float) -> Image.Image:
    im = Image.new("RGB", (W, H), C["bg"])
    d = ImageDraw.Draw(im)
    scene = min(3, int(t // 5))
    phase = (t % 5) / 5

    def text(x, y, value, size=22, color="fg", bold=False, mono=False):
        d.text((x, y), value, font=font(size, bold, mono), fill=C.get(color, color))

    def box(x, y, width, height, fill="panel", outline="line", radius=14):
        d.rounded_rectangle((x, y, x + width, y + height), radius,
                            fill=C.get(fill, fill), outline=C.get(outline, outline), width=2)

    def line(points, color="line", width=3):
        d.line(points, fill=C.get(color, color), width=width, joint="curve")

    def pill(x, y, label, color="cyan"):
        width = int(d.textlength(label, font=font(15, True))) + 26
        box(x, y, width, 29, "raised", color, 14)
        text(x + 13, y + 4, label, 15, color, True)

    def dot(x, y, color="cyan", radius=5):
        d.ellipse((x-radius, y-radius, x+radius, y+radius), fill=C[color])

    # Presentation chrome intentionally differs from the real desktop UI.
    box(44, 32, 56, 56, "raised", "cyan", 14)
    text(55, 40, "Pi", 30, "cyan", True)
    text(117, 28, "Pi Desktop", 32, bold=True)
    text(119, 71, "A workspace for coding-agent sessions", 18, "dim")
    pill(960, 47, "ILLUSTRATED WALKTHROUGH", "dim")

    titles = ["Your sessions, connected.", "Inspect the work behind a reply.",
              "Follow a child. Keep the context.", "See runtime status in context."]
    subtitles = ["Project grouping and parent / child relationships",
                 "Messages, tool calls and their results in one conversation",
                 "Open a subagent conversation from its parent session",
                 "Execution state and terminal location are separate signals"]
    text(46, 123, f"0{scene + 1}", 34, "cyan", True)
    text(112, 119, titles[scene], 38, bold=True)
    text(113, 174, subtitles[scene], 20, "dim")

    # The same synthetic project and three sessions anchor every chapter.
    box(44, 226, 306, 351)
    text(64, 246, "PROJECT", 13, "dim", True)
    text(64, 274, "research-toolkit", 22, bold=True)
    text(64, 309, "3 sessions  /  2 subagents", 16, "dim")
    line([(64, 347), (330, 347)], width=1)
    rows = [(367, "Research workflow", "Parent conversation", "cyan"),
            (430, "Review result schema", "Child conversation", "purple"),
            (493, "Document task runner", "Child  /  RMUX", "green")]
    active = 1 if scene == 2 else 2 if scene == 3 else 0
    for i, (y, title, desc, color) in enumerate(rows):
        if i == active:
            box(57, y-8, 279, 61, "raised", color, 8)
        dot(72, y+10, color, 4)
        text(87, y-2, title, 18, color if i == active else "fg", i == active)
        text(87, y+26, desc, 13, "dim")

    if scene == 0:
        # A relationship diagram, not a fabricated application screenshot.
        box(463, 240, 661, 95, outline="cyan")
        pill(483, 257, "PARENT")
        text(597, 265, "Prepare a research workflow", 23, bold=True)
        line([(793, 335), (793, 380), (592, 380), (592, 420)], "purple")
        line([(793, 380), (1000, 380), (1000, 420)], "purple")
        distance = int(min(1, phase * 1.6) * 80)
        dot(793, 335 + min(distance, 44), "cyan", 6)
        for x, title, desc, color in [(393, "Review result schema", "Inspect a child conversation", "purple"),
                                       (813, "Document task runner", "Identify the running worker", "green")]:
            box(x, 421, 385, 136, outline=color)
            text(x+22, 441, "SUBAGENT", 13, color, True)
            text(x+22, 474, title, 22, bold=True)
            text(x+22, 514, desc, 16, "dim")
    elif scene == 1:
        box(378, 226, 858, 351)
        text(402, 244, "Prepare a research workflow", 22, bold=True)
        line([(402, 282), (1212, 282)], width=1)
        text(404, 302, "USER", 13, "cyan", True)
        text(494, 299, "Keep task status separate from research results.", 20)
        text(404, 346, "AGENT", 13, "purple", True)
        text(494, 343, "Review the schema in a separate child session.", 20)
        expanded = phase >= .20
        box(402, 389, 808, 113 if expanded else 54, "raised", "purple", 9)
        text(420, 403, "v" if expanded else ">", 19, "purple", mono=True)
        text(453, 403, "TOOL CALL", 15, "purple", True)
        text(591, 400, "subagent", 21, mono=True)
        if expanded:
            text(425, 441, 'agent: reviewer', 17, "dim", mono=True)
            text(425, 470, 'task:  Review the sample schema', 17, mono=True)
        if phase >= .46:
            dot(417, 539, "green", 5)
            text(435, 524, "RESULT", 13, "green", True)
            text(524, 520, "Task ID, status and results are separate fields.", 19)
    elif scene == 2:
        box(378, 226, 858, 351, outline="purple")
        text(402, 247, "Research workflow", 17, "dim")
        text(599, 247, "/", 17, "purple")
        text(620, 247, "Review result schema", 17, "purple", True)
        line([(402, 285), (1212, 285)], width=1)
        pill(402, 304, "CHILD CONVERSATION", "purple")
        text(402, 351, "Review the result schema", 29, bold=True)
        text(404, 405, "Reviewed the sample schema:", 21)
        for i, label in enumerate(["Task ID", "Execution status", "Result fields"]):
            x = 404 + i * 266
            box(x, 455, 249, 60, "raised", "line", 9)
            dot(x+20, 485, "green", 4)
            text(x+36, 471, label, 19)
        text(404, 538, "The parent relationship stays visible in the session list.", 17, "dim")
    else:
        box(378, 226, 858, 351, outline="green")
        text(402, 245, "Document the task runner", 26, bold=True)
        text(404, 287, "Child session of: Prepare a research workflow", 17, "dim")
        box(402, 336, 383, 143, "raised", "line", 10)
        box(805, 336, 407, 143, "raised", "line", 10)
        text(424, 355, "EXECUTION STATE", 14, "dim", True)
        text(827, 355, "TERMINAL LOCATION", 14, "dim", True)
        dot(438, 424, "green", 7)
        text(463, 401, "Running", 32, "green", True)
        text(827, 401, "RMUX", 32, "cyan", True)
        text(424, 496, "Illustrated status from a synthetic session.", 20)
        text(424, 535, "No worker or terminal is started in this animation.", 17, "dim")

    # Chapter bar, with an unmistakable disclosure on every frame.
    for i, label in enumerate(["Projects", "Tool calls", "Child sessions", "Runtime"]):
        x = 46 + i * 307
        line([(x, 612), (x+280, 612)], "line", 3)
        if i < scene:
            line([(x, 612), (x+280, 612)], "cyan", 3)
        elif i == scene:
            line([(x, 612), (x+max(1, round(280*phase)), 612)], "cyan", 3)
        text(x, 627, label, 18, "cyan" if i == scene else "dim", i == scene)
    text(46, 679, "Synthetic data  /  Concept animation, not a screen recording", 16, "dim")
    text(916, 679, "roshameow/pi-session-viewer", 16, "dim")
    return im


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "docs/assets")
    parser.add_argument("--qa", type=Path, help="Optional contact sheet destination")
    args = parser.parse_args()
    if not shutil.which("ffmpeg"):
        raise SystemExit("Install ffmpeg before rendering.")
    out = args.output
    out.mkdir(parents=True, exist_ok=True)
    poster = frame(2.5)
    poster.save(out / "walkthrough-illustrated.png", optimize=True)

    # H.264 is widely playable; there is intentionally no audio track.
    command = ["ffmpeg", "-y", "-loglevel", "error", "-f", "rawvideo", "-pixel_format", "rgb24",
               "-video_size", f"{W}x{H}", "-framerate", str(FPS), "-i", "pipe:0", "-an",
               "-c:v", "libx264", "-threads", "2", "-preset", "medium", "-crf", "20",
               "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(out / "walkthrough-illustrated.mp4")]
    palette_sheet = Image.new("RGB", (W, H*4))
    for i in range(4):
        palette_sheet.paste(frame(i*5+2.5), (0, i*H))
    palette = palette_sheet.quantize(colors=128)
    gif_frames = []
    proc = subprocess.Popen(command, stdin=subprocess.PIPE)
    try:
        assert proc.stdin is not None
        for n in range(FPS*SECONDS):
            im = frame(n/FPS)
            proc.stdin.write(im.tobytes())
            if n % 2 == 0:
                gif_frames.append(im.quantize(palette=palette, dither=Image.Dither.NONE))
    finally:
        if proc.stdin:
            proc.stdin.close()
    if proc.wait() != 0:
        raise SystemExit("ffmpeg could not encode the video.")
    # GIF delays are measured in centiseconds: 170+160+170 = 500 ms.
    durations = [170, 160, 170] * (len(gif_frames)//3)
    gif_frames[0].save(out / "walkthrough-illustrated.gif", save_all=True,
                       append_images=gif_frames[1:], duration=durations, loop=0,
                       optimize=True, disposal=1)
    if args.qa:
        args.qa.parent.mkdir(parents=True, exist_ok=True)
        sheet = Image.new("RGB", (W, H), C["bg"])
        for i in range(4):
            thumb = frame(i*5+3).resize((W//2, H//2), Image.Resampling.LANCZOS)
            sheet.paste(thumb, ((i % 2)*(W//2), (i//2)*(H//2)))
        sheet.save(args.qa)
    for path in sorted(out.glob("walkthrough-illustrated.*")):
        print(f"{path.relative_to(ROOT) if path.is_relative_to(ROOT) else path}: {path.stat().st_size:,} bytes")


if __name__ == "__main__":
    main()
