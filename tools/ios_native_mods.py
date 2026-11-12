#!/usr/bin/env python3
"""Native mods for the iOS app (docs/ios.md): the mods a build is made with, fetched and checked.

An iPhone runs no code its app did not ship with, so a native mod is compiled into the app itself:
`.github/workflows/ios.yml`, run with mod ids, fetches them here, applies them to Starling's source
(packaging/ios/native_stage.sh, as Starling's own mod build does) and builds the .ipa from that. The
game reads nothing of a mod's file at run time (Android's build leaves its assets/ and data/ out too):
its code is all of it.

    ios_native_mods.py fetch INDEX OUT ID...   # each mod's horizon-mixins bundle into OUT/bundles/<id>/;
                                               # OUT/fetched.json says what went in
    ios_native_mods.py info OUT COMMIT         # the app's native-mods.json, on standard output
    ios_native_mods.py names OUT               # the mods fetched, "id version, id version"
    ios_native_mods.py list INDEX              # every native mod there is, one line each

A mod id names a folder of native-mods/ in this repository (Starling's own, read before they were
added), else a native mod of Horizon's registry (INDEX, its index.json) for this game version. Only
the registry's own files are downloaded (GitHub release assets over https), each checked against the
size and SHA-256 the registry gives before it is opened; nothing in them is run here. The checks of
a bundle's files are the build service's (packaging/android/mod_build_server.py `read_bundle`).

The registry fetch follows meehandaniel4/horizon-iOS's packaging/ios/native/native_mods.py (MIT).
"""
import hashlib
import json
import pathlib
import re
import shutil
import sys
import urllib.request
import zipfile

GAME = "26.3"
REPO = pathlib.Path(__file__).resolve().parent.parent
MAX_FILE = 64 << 20
MAX_EXPANDED = 256 << 20
MAX_ENTRIES = 4096
MAX_MODS = 32
ID = re.compile(r"^[a-z0-9][a-z0-9_-]{0,63}$")


class Refused(Exception):
    pass


def native(index):
    """The registry's native mods with a version for this game, by id."""
    out = {}
    for project in index.get("projects", []):
        if project.get("kind") != "native-mod":
            continue
        versions = [v for v in project.get("versions", []) if v.get("game") == GAME and not v.get("revoked")]
        if versions:
            out[project["id"]] = (project, versions[0])
    return out


def allowed(rest):
    """As horizon_mods::index::native::allowed: sources, manifests, data, a crate's text files."""
    parts = rest.split("/")
    name = parts[-1]
    in_data = len(parts) >= 2 and (parts[0] == "data" or (len(parts) >= 3 and parts[1] == "data"))
    text = len(parts) <= 2 and (
        name.startswith(("README", "LICENSE", "LICENCE", "COPYING", "NOTICE")) or name == ".gitignore"
    )
    return rest.endswith((".rs", ".toml")) or in_data or text


def read_bundle(path, expected):
    """The entries of the bundle in the .hnmod at `path`, checked. Nothing is written or run."""
    try:
        archive = zipfile.ZipFile(path)
    except zipfile.BadZipFile as e:
        raise Refused(f"not a native mod: {e}")
    with archive:
        infos = [i for i in archive.infolist() if not i.is_dir()]
        if len(infos) > MAX_ENTRIES or sum(i.file_size for i in infos) > MAX_EXPANDED:
            raise Refused("too big")
        for info in infos:
            parts = info.filename.split("/")
            mode = info.external_attr >> 16
            if (
                "\\" in info.filename
                or ":" in info.filename
                or any(p in ("", ".", "..") for p in parts)
                or (mode & 0o170000 and mode & 0o170000 != 0o100000)
            ):
                raise Refused(f"bad path {info.filename[:80]}")
        try:
            manifest = json.loads(archive.read("horizon-native.json"))
            mod_id = manifest["id"]
        except (KeyError, ValueError, TypeError):
            raise Refused("no readable horizon-native.json")
        if mod_id != expected:
            raise Refused(f"it is the mod {str(mod_id)[:64]}, not {expected}")
        prefix = f"mixins/{mod_id}/"
        entries = {}
        for info in infos:
            name = info.filename
            if name in ("horizon-native.json", "horizon-mod.toml") or name.startswith(("assets/", "data/")):
                continue
            if not name.startswith(prefix) or not allowed(name[len(prefix):]):
                raise Refused(f"unexpected file {name[:80]}")
            entries[name[len(prefix):]] = archive.read(info)
        if "horizon-mixin.toml" not in entries:
            raise Refused("no horizon-mixin.toml")
        return entries


def download(url, size, sha):
    if not url.startswith("https://github.com/") or "/releases/download/" not in url or size > MAX_FILE:
        raise Refused(f"unexpected file {url} ({size} bytes)")
    request = urllib.request.Request(url, headers={"User-Agent": "starling-ios-native-build"})
    with urllib.request.urlopen(request, timeout=180) as response:
        data = response.read(MAX_FILE + 1)
    if len(data) != size or hashlib.sha256(data).hexdigest() != sha:
        raise Refused("its size or SHA-256 is not the registry's")
    return data


def version_of(bundle):
    """The `version` of a bundle's `[mod]` table (no TOML reader: any Python 3 runs this)."""
    text = (bundle / "horizon-mixin.toml").read_text(encoding="utf-8")
    table = re.search(r'(?ms)^\[mod\]\s*$(.*?)(?=^\[)', text + "\n[")
    found = table and re.search(r'(?m)^version\s*=\s*"([^"]*)"', table.group(1))
    return found.group(1) if found else "?"


def fetch(index, out, ids):
    registry = native(index)
    bundles = out / "bundles"
    shutil.rmtree(out, ignore_errors=True)
    bundles.mkdir(parents=True)
    report = {"fetched": [], "refused": []}
    ids = sorted(set(ids))
    if len(ids) > MAX_MODS:
        raise SystemExit(f"{len(ids)} mods: at most {MAX_MODS} go in one build")
    for mod in ids:
        try:
            if not ID.match(mod):
                raise Refused("not a mod id")
            own = REPO / "native-mods" / mod / "mixins" / mod
            dest = bundles / mod
            if (own / "horizon-mixin.toml").is_file():
                shutil.copytree(own, dest, symlinks=False)
                report["fetched"].append({"id": mod, "version": version_of(dest), "from": "native-mods/"})
                continue
            if mod not in registry:
                raise Refused(f"not in native-mods/, and no native mod of the registry for {GAME}")
            project, version = registry[mod]
            file = version["file"]
            data = download(file["url"], int(file["size"]), file["sha256"])
            archive = out / f".{mod}.hnmod"
            archive.write_bytes(data)
            try:
                entries = read_bundle(archive, mod)
            finally:
                archive.unlink()
            for rest, content in entries.items():
                target = dest / rest
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(content)
            report["fetched"].append({
                "id": mod,
                "version": version["version"],
                "name": project.get("name", mod),
                "from": "registry",
                "sha256": file["sha256"],
            })
        except Refused as e:
            report["refused"].append({"id": mod, "why": str(e)})
        except OSError as e:
            report["refused"].append({"id": mod, "why": f"could not be fetched: {e}"})
    (out / "fetched.json").write_text(json.dumps(report, indent=1) + "\n", encoding="utf-8")
    return report


def info(out, commit):
    """What the game reads beside its program (game/mods.rs BUILD_INFO): the mods it was built with."""
    report = json.loads((out / "fetched.json").read_text(encoding="utf-8"))
    return {
        "commit": commit,
        "mods": [{"id": m["id"], "version": m["version"]} for m in report["fetched"]],
    }


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    command, *args = sys.argv[1:]
    if command == "fetch" and len(args) >= 3:
        index = json.loads(pathlib.Path(args[0]).read_text(encoding="utf-8"))
        report = fetch(index, pathlib.Path(args[1]), args[2:])
        print(json.dumps(report, indent=1))
        if report["refused"] or not report["fetched"]:
            for refused in report["refused"]:
                print(f"::error::{refused['id']}: {refused['why']}")
            raise SystemExit(1)
    elif command == "info" and len(args) == 2:
        print(json.dumps(info(pathlib.Path(args[0]), args[1]), indent=1))
    elif command == "names" and len(args) == 1:
        mods = info(pathlib.Path(args[0]), "")["mods"]
        print(", ".join(f"{m['id']} {m['version']}" for m in mods))
    elif command == "list" and len(args) == 1:
        index = json.loads(pathlib.Path(args[0]).read_text(encoding="utf-8"))
        own = sorted(p.name for p in (REPO / "native-mods").iterdir() if (p / "mixins").is_dir())
        for mod in own:
            print(f"{mod}\tnative-mods/")
        for mod, (project, version) in sorted(native(index).items()):
            if mod not in own:
                summary = " ".join(str(project.get("summary", "")).split())
                print(f"{mod}\t{project.get('name', mod)} {version['version']}\t{summary}")
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
