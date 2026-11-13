#!/usr/bin/env python3
"""The pinned official game files for a CI check, in the official launcher's layout: prints the
server jar's path (tools/ios_prepare.py's download, each file checked against research-lock.json)."""
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools'))
import ios_prepare  # noqa: E402
import lab  # noqa: E402

target = Path(sys.argv[1]).resolve()
server = ios_prepare.download_game(lab.pinned_version(), target)
print(server)
