# Illustrated walkthrough

[MP4](assets/walkthrough-illustrated.mp4) · [GIF](assets/walkthrough-illustrated.gif) · [Still image](assets/walkthrough-illustrated.png)

This 20-second, silent animation explains project grouping, tool calls, child sessions and runtime indicators. It is a **concept animation, not an application screen recording**. Its simplified cards illustrate relationships; they do not reproduce or verify the desktop layout, live execution or terminal attachment. This disclosure is also visible throughout the animation.

The palette comes from `src/styles.css`; project and session examples come from `src/demo/fixtures.ts`. No local session files, account data, model calls, workers or terminals are used. The browser demo described in the README renders the actual UI components with synthetic data.

## Rebuild the assets

Requirements: Python 3.10+, Pillow, FFmpeg with the `libx264` encoder, and DejaVu Sans / DejaVu Sans Mono fonts.

```bash
python -m pip install Pillow==12.3.0
python scripts/render_walkthrough.py
```

Fonts default to `/usr/share/fonts/truetype/dejavu`. Set `PI_DEMO_FONT_DIR` to the directory containing the DejaVu `.ttf` files on another system. Fonts are required for rendering but are not bundled in this repository.

The script writes the MP4, looping GIF and still image to `docs/assets/`. Pass `--qa /tmp/pi-walkthrough-contact-sheet.png` to produce a four-chapter contact sheet for visual review. The normal application build does not depend on this script or its Python dependencies.
