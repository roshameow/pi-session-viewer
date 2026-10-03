#!/usr/bin/env python3
"""Offline CLI fixture only: no Pi imports, networking, model or MCP calls."""
import json
import os
import sys

with open(".pi/mcp.json", encoding="utf-8") as config:
    marker = json.load(config)["fixtureMarker"]
print(json.dumps({"type": "fake_config", "cwd": os.getcwd(), "marker": marker, "argv": sys.argv[1:]}), flush=True)
# Well beyond a normal OS pipe buffer. Completion requires concurrent stderr drain.
for _ in range(1024):
    sys.stderr.write("mock-native-startup:" + "x" * 1024 + "\n")
sys.stderr.flush()
print(json.dumps({"type": "agent_settled", "fixture": True}), flush=True)
