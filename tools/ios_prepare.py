#!/usr/bin/env python3
"""Starling's game files for the iOS app, made in CI (IOS-SYNC.md, crates/horizon-ios-prepare).

Starling's Android app prepares the game on the phone: its download of the official files, then
horizon-prepare with a Java runtime. iOS lets an app run no Java, so this runs the same preparer
here, once, and the app carries what it made, without Mojang's own files:

1. the pinned official files from Mojang, each checked against research-lock.json and Mojang's
   hashes, in the official launcher's layout (what the phone's own download makes);
2. horizon-prepare over them, as the phone runs it (--prepare --complete), on a case-sensitive
   volume as a phone's file system is;
3. horizon-ios-prepare pack: the tables it generated into engine-data.zip, with the recipe that
   lays Mojang's files out again from the player's own download;
4. horizon-ios-prepare check: that layout, done here as the phone does it, is horizon-prepare's
   folder file for file, and the zip holds none of Mojang's files;
5. the other files the app carries, zipped as packaging/android/build_apk.py zips them for the APK
   (lowbudget.zip, StarlingVisuals.zip).

Everything lands in .local/ios/assets, which packaging/ios/project.yml copies into the app.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
sys.path.insert(0, str(ROOT / 'packaging' / 'android'))
import lab  # noqa: E402

MANIFEST = 'https://piston-meta.mojang.com/mc/game/version_manifest_v2.json'
OBJECTS = 'https://resources.download.minecraft.net'


def fetch_json(url):
    import urllib.request
    request = urllib.request.Request(url, headers={'User-Agent': 'horizon/0.1'})
    with urllib.request.urlopen(request, timeout=120) as response:
        return json.load(response)


def inside(base, value):
    path = (base / value).resolve()
    if not path.is_relative_to(base.resolve()):
        raise ValueError(f'Path escapes the download folder: {value}')
    return path


def parallel_download(files):
    def one(item):
        url, path, checksum = item
        for attempt in range(4):
            try:
                lab.download(url, path, checksum)
                return
            except (OSError, ValueError):
                if attempt == 3:
                    raise
                time.sleep(1 + attempt)
    with ThreadPoolExecutor(max_workers=8) as pool:
        for _ in pool.map(one, files):
            pass


def download_game(version, directory):
    """The pinned version's files in the official launcher's layout; returns the server jar."""
    pins = json.loads((ROOT / 'research-lock.json').read_text())['minecraft']
    if version != pins['version']:
        raise ValueError(f"This engine implements {pins['version']}; {version} is unsupported")
    entry = next((v for v in fetch_json(MANIFEST)['versions'] if v['id'] == version), None)
    if entry is None:
        raise ValueError(f'Mojang does not list Minecraft {version}')
    versions = directory / 'versions' / version
    manifest_path = versions / f'{version}.json'
    lab.download(entry['url'], manifest_path, entry['sha1'])
    manifest = json.loads(manifest_path.read_text())
    files = []
    for side in ['client', 'server']:
        artifact = manifest['downloads'][side]
        if artifact['sha1'] != pins[f'{side}_sha1']:
            raise ValueError(f'The {side} jar differs from the engine pin')
        name = f'{version}-server.jar' if side == 'server' else f'{version}.jar'
        files.append((artifact['url'], versions / name, artifact['sha1']))
    for artifact in lab.client_library_artifacts(manifest):
        files.append((artifact['url'], inside(directory / 'libraries', artifact['path']), artifact['sha1']))
    index = manifest['assetIndex']
    index_path = inside(directory / 'assets/indexes', f"{index['id']}.json")
    lab.download(index['url'], index_path, index['sha1'])
    for value in json.loads(index_path.read_text())['objects'].values():
        checksum = value['hash']
        name = f'{checksum[:2]}/{checksum}'
        files.append((f'{OBJECTS}/{name}', directory / 'assets/objects' / name, checksum))
    print(f'Downloading Minecraft {version}: {len(files)} checked files', flush=True)
    parallel_download(files)
    return versions / f'{version}-server.jar'


def cargo(*args, env=None):
    print('+ cargo', ' '.join(str(a) for a in args), flush=True)
    subprocess.run(['cargo', *[str(a) for a in args]], cwd=ROOT, env=env, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--java-home', type=Path, required=True, help='A JDK of the pinned version')
    parser.add_argument('--minecraft', type=Path, required=True,
                        help='Where the official files go (kept between runs: each is checked again)')
    parser.add_argument('--work', type=Path, required=True,
                        help='A folder on a case-sensitive volume for the preparation')
    args = parser.parse_args()
    version = lab.pinned_version()
    work = args.work.resolve()
    minecraft = args.minecraft.resolve()
    resources = work / 'resources'
    assembled = work / 'assembled'
    out = ROOT / '.local' / 'ios' / 'assets'
    out.mkdir(parents=True, exist_ok=True)
    work.mkdir(parents=True, exist_ok=True)

    server = download_game(version, minecraft)

    environment = dict(os.environ, JAVA_HOME=str(args.java_home.resolve()))
    isolated = Path(tempfile.mkdtemp(prefix='prepare-', dir=work))
    for key in ['HORIZON_HOME', 'HORIZON_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME']:
        environment[key] = str(isolated / key.lower())
    environment.update(HORIZON_LAUNCHER_NO_SELF_UPDATE='1')
    cargo('run', '--release', '-p', 'horizon-prepare', '--', '--root', ROOT, '--prepare', '--complete',
          '--minecraft-dir', minecraft, '--server-jar', server, '--java-home', args.java_home,
          '--output', resources, env=environment)

    engine = out / 'engine-data.zip'
    cargo('run', '--release', '-p', 'horizon-ios-prepare', '--', 'pack', '--resources', resources,
          '--minecraft', minecraft, '--version', version, '--out', engine)
    cargo('run', '--release', '-p', 'horizon-ios-prepare', '--', 'check', '--engine', engine,
          '--minecraft', minecraft, '--reference', resources, '--version', version, '--work', assembled)

    import build_apk  # noqa: E402  (its zips, byte for byte the APK's)
    build_apk.zip_folder(ROOT / 'packaging' / 'lowbudget' / 'pack', out / 'lowbudget.zip')
    build_apk.zip_folder(ROOT / 'packaging' / 'shaderpacks' / 'starling-visuals', out / 'StarlingVisuals.zip')
    for name in sorted(p.name for p in out.iterdir()):
        print(f'iOS app asset: {name} ({(out / name).stat().st_size // 1024} KiB)', flush=True)


if __name__ == '__main__':
    main()
