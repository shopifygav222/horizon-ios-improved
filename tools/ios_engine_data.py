#!/usr/bin/env python3
"""Package the generated engine tables for the iOS app bundle, exactly as
`tools/android_build.py --public` packages them for the APK: only tables Horizon
generates, never Mojang's own files.

Run tools/android_prepare.py first; packaging/ios/project.yml copies the result
into the app.
"""
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import android_build  # noqa: E402


def main():
    output = android_build.engine_data(public=True)
    with zipfile.ZipFile(output) as archive:
        names = [n for n in archive.namelist() if not n.endswith('/')]
    foreign = [n for n in names if not android_build.generated_engine_file(n)]
    if not names or foreign:
        raise SystemExit(f'{output} holds files that are not generated engine tables: {foreign[:5]}')
    print(f'iOS engine data: {output} ({len(names)} generated tables)', flush=True)


if __name__ == '__main__':
    main()
