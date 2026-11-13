#!/usr/bin/env python3
"""Shared worlds, phase 0: do two different builds of the engine play and save together?

A shared world moves between its players' games: one hosts it, a friend joins, and later the
friend's game hosts the same save. Before anything is built on that, this proves it for two
engine builds (this port, and the official Horizon a PC friend runs), with no shared-worlds code
involved:

1. play:   build A hosts a new world behind an invite code (Play with Friends), build B joins
           with the code: chat, movement and blocks both ways (tools/test_friends.py's checks);
2. save:   A closes the world, and every chunk in every region file reads back;
3. move:   the world folder is copied to B, which opens it: the block A placed is there. B places
           its own, closes, and every chunk reads back;
4. return: the folder is copied back to A, which opens it and finds both blocks.

    python3 tools/shared_worlds_compat.py --a-client A --a-resources RA --b-client B --b-resources RB --xvfb

Run it both ways round (swap a and b). Nothing leaves the machine: no router is asked for a port
and STUN is off.
"""
import argparse
import gzip
import math
import shutil
import sys
import tempfile
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import test_friends as tf  # noqa: E402

SECTOR = 4096
ENV = {'HORIZON_FRIENDS_UPNP': '0', 'HORIZON_STUN_SERVERS': 'none'}
WORLD = 'Friends'


def check_regions(world):
    """Every stored chunk of every region file: inside the file, and its bytes decompress to a
    compound. Returns how many chunks were read."""
    chunks = 0
    files = sorted(world.rglob('*.mca'))
    assert files, f'{world} has no region files'
    for path in files:
        data = path.read_bytes()
        if not data:
            continue
        assert len(data) >= 2 * SECTOR, f'{path}: {len(data)} bytes is shorter than a header'
        for index in range(1024):
            location = int.from_bytes(data[index * 4:index * 4 + 4], 'big')
            if location == 0:
                continue
            offset, sectors = (location >> 8) * SECTOR, (location & 0xFF) * SECTOR
            where = f'{path} chunk {index}'
            assert offset >= 2 * SECTOR and offset + sectors <= len(data), f'{where}: sectors outside the file'
            length = int.from_bytes(data[offset:offset + 4], 'big')
            kind = data[offset + 4]
            assert 1 <= length <= sectors - 4, f'{where}: length {length} in {sectors} bytes'
            payload = data[offset + 5:offset + 4 + length]
            if kind & 0x80:
                x, z = (int(v) for v in path.stem.split('.')[1:3])
                external = path.with_name(f'c.{x * 32 + index % 32}.{z * 32 + index // 32}.mcc')
                assert external.is_file(), f'{where}: {external.name} is missing'
                payload = external.read_bytes()
                kind &= 0x7F
            if kind == 1:
                body = gzip.decompress(payload)
            elif kind == 2:
                body = zlib.decompress(payload)
            elif kind == 3:
                body = payload
            else:
                assert kind == 4, f'{where}: unknown compression {kind}'
                body = None
            assert body is None or (body and body[0] == 10), f'{where}: not a compound'
            chunks += 1
    return chunks


def close_world(game):
    game('disconnect')
    game.wait('screen', lambda v: v['screen'] == 'title', 120, 'the title after closing')


def open_world(game):
    game(f'open {WORLD} 0 creative')
    return game.wait('status', lambda v: v['player'] is not None and v['pending_chunks'] == 0, 180, 'the world open')


def block_is(game, pos, name, what):
    game.wait(f'getblock {pos[0]} {pos[1]} {pos[2]}', lambda v: v['block'] == name, 30, what)


def tails(games):
    for game in games:
        print(f'--- {game.home.name} log tail ---')
        print('\n'.join(game.stop().splitlines()[-40:]))


def run(a, b, work):
    made = []

    def game(side, name):
        g = tf.Game(side[0], side[1], work, name, ENV)
        made.append(g)
        return g

    try:
        # 1. Play together: A hosts, B joins by code.
        host, guest = game(a, 'a1'), game(b, 'b1')
        code = tf.host_world(host)
        tf.join(guest, code, 'Remote')
        tf.play_together(host, guest, 'Remote', 1)
        hp = host('status')['player']['position']
        first = [math.floor(hp[0]) + 2, math.floor(hp[1]) + 5, math.floor(hp[2])]
        second = [first[0], first[1] + 2, first[2]]
        host(f'command setblock {first[0]} {first[1]} {first[2]} minecraft:diamond_block')
        block_is(guest, first, 'minecraft:diamond_block', 'the block of A on B, over the tunnel')
        tf.leave(host, guest, 'Remote')
        print('play: B joined A by invite code; chat, movement and blocks both ways', flush=True)
        # 2. The save A wrote reads back.
        close_world(host)
        host.stop()
        guest.stop()
        saved = host.home / 'saves' / WORLD
        print(f'save: A wrote {check_regions(saved)} chunks, all readable', flush=True)
        # 3. B opens the world A wrote.
        other = game(b, 'b2')
        shutil.copytree(saved, other.home / 'saves' / WORLD, ignore=shutil.ignore_patterns('session.lock'))
        open_world(other)
        block_is(other, first, 'minecraft:diamond_block', 'the block of A in the world B opened')
        other(f'command setblock {second[0]} {second[1]} {second[2]} minecraft:emerald_block')
        block_is(other, second, 'minecraft:emerald_block', 'the block of B')
        close_world(other)
        other.stop()
        moved = other.home / 'saves' / WORLD
        print(f'move: B opened it, found the block and wrote {check_regions(moved)} chunks, all readable', flush=True)
        # 4. And back to A.
        back = game(a, 'a2')
        shutil.copytree(moved, back.home / 'saves' / WORLD, ignore=shutil.ignore_patterns('session.lock'))
        open_world(back)
        block_is(back, first, 'minecraft:diamond_block', 'the block of A after the round trip')
        block_is(back, second, 'minecraft:emerald_block', 'the block of B in the world A opened again')
        close_world(back)
        back.stop()
        final = check_regions(back.home / 'saves' / WORLD)
        print(f'return: A opened it again with both blocks, {final} chunks readable', flush=True)
    except BaseException:
        tails(made)
        raise
    finally:
        for g in made:
            g.stop()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for side in 'ab':
        parser.add_argument(f'--{side}-client', type=Path, required=True)
        parser.add_argument(f'--{side}-resources', type=Path, required=True)
    parser.add_argument('--offscreen', action='store_true', help='run the games without a window')
    parser.add_argument('--xvfb', action='store_true', help='run the games under xvfb-run (headless Linux)')
    args = parser.parse_args()
    tf.Game.prefix = ['xvfb-run', '-a'] if args.xvfb else []
    tf.Game.extra = ['--offscreen'] if args.offscreen else []
    a = (args.a_client.resolve(), args.a_resources.resolve())
    b = (args.b_client.resolve(), args.b_resources.resolve())
    # Short: Unix socket paths are limited to about 100 bytes.
    with tempfile.TemporaryDirectory(prefix='hsw-', dir='/tmp') as directory:
        run(a, b, Path(directory))
    print('Shared worlds compatibility passed: play by code, and the save both ways')


if __name__ == '__main__':
    main()
