#!/usr/bin/env python3
"""Shared worlds end to end: two real games and a real S3 server (MinIO), on one machine.

1. create:   Alex makes a world, shares it into the bucket; Steve adds it from Alex's invite.
2. host:     Alex opens it (nobody is playing, so Alex hosts); the list says one is playing.
3. join:     Steve opens it and joins Alex through the invite code the lease holds; a block Alex
             places shows on Steve's game; a checkpoint uploads.
4. give:     Alex gives the world to Steve: Alex's world closes and uploads, Steve's game takes
             it and opens it with Alex's block, and Alex's game joins Steve's by itself.
5. takeover: Steve places a block, takes a checkpoint, and Steve's game is killed. Alex's game
             waits for the lease to stand still, takes the world from that checkpoint (the
             block is there) and hosts it.
6. close:    Alex leaves: the last copy uploads and the world is closed; every chunk of every
             region file reads back.

    python3 tools/shared_worlds_e2e.py --client target/fast/horizon-client --resources R --xvfb --offscreen

The SHARED_WORLDS_S3_* variables name the bucket (tools/ci/s3_server.sh starts one). Nothing
else leaves the machine: no router is asked for a port, STUN and the public tunnel are off.
The players are offline players (HORIZON_SHARED_WORLDS_TEST_PLAYER), as CI has no accounts.
"""
import argparse
import math
import os
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
import shared_worlds_compat as compat  # noqa: E402
import test_friends as tf  # noqa: E402
from integration_env import game_environment  # noqa: E402

WORLD = 'Island'
BASE_ENV = {
    'HORIZON_FRIENDS_UPNP': '0',
    'HORIZON_STUN_SERVERS': 'none',
    'HORIZON_FRIENDS_LOOPBACK': '1',
    'HORIZON_SHARED_TUNNEL': 'none',
}


class Player:
    prefix = []
    extra = []

    def __init__(self, client, resources, work, name):
        self.name = name
        self.uuid = lab.offline_uuid(name).hex
        self.home = work / name.lower()
        self.home.mkdir(exist_ok=True)
        self.endpoint = self.home / 'c.sock'
        if self.endpoint.exists():
            self.endpoint.unlink()
        self.log = (self.home / 'client.log').open('a+')
        env = {**os.environ, **BASE_ENV, 'HORIZON_SHARED_WORLDS_TEST_PLAYER': name}
        self.process = subprocess.Popen(Player.prefix + [
            str(client), '--resources', str(resources), '--render-distance', '2', '--test-terrain',
            '--name', name, '--control-socket', str(self.endpoint), '--timeout-seconds', '1800',
        ] + Player.extra, cwd=self.home,
            env={**game_environment(env, self.home), 'HORIZON_HOME': str(self.home)},
            stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        self.wait('screen', lambda v: v['screen'] == 'title', 120, f'{name}: the title')

    def __call__(self, line, check=True):
        return tf.command(self.endpoint, line, check)

    def wait(self, line, predicate, timeout=60, what=None):
        return tf.wait_for(self.endpoint, line, predicate, timeout, what)

    def shared(self):
        return self('shared status')

    def session(self):
        return self.shared().get('session') or {}

    def wait_task(self, done, timeout, what):
        return self.wait('shared status', lambda v: done(v['task']), timeout, what)

    def wait_hosting(self, timeout, what):
        return self.wait('shared status', lambda v: (v.get('session') or {}).get('phase') == 'hosting'
                         and v['session']['opened'] and v['session']['code'], timeout, what)['session']

    def wait_joined(self, timeout, what):
        self.wait('shared status', lambda v: (v.get('session') or {}).get('phase') == 'watching'
                  and v['session']['joined'], timeout, what)
        return self.wait('status', lambda v: v['server'] and v['server']['joined'] and v['player'] is not None,
                         timeout, what)

    def block(self, pos, name, what, timeout=30):
        self.wait(f'getblock {pos[0]} {pos[1]} {pos[2]}', lambda v: v['block'] == name, timeout, what)

    def checkpoint(self, what):
        """A checkpoint, taken again while one is put off (a write came in between)."""
        for _ in range(10):
            if self('shared checkpoint')['taken'] is True:
                break
            time.sleep(1)
        else:
            raise AssertionError(f'{self.name}: no checkpoint was taken')
        self.wait('shared status', lambda v: not v['session']['uploading'] and v['session']['committed'],
                  120, what)

    def kill(self):
        """The game dies at once, as a phone that runs out of battery: the whole process group,
        not only xvfb-run around it."""
        os.killpg(self.process.pid, signal.SIGKILL)
        self.process.wait(timeout=20)

    def stop(self):
        if self.process.poll() is None:
            try:
                self('quit', check=False)
                self.process.wait(timeout=60)
            except Exception:
                self.process.terminate()
                try:
                    self.process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self.process.kill()
        self.log.seek(0)
        return self.log.read()


def bucket():
    names = ['SHARED_WORLDS_S3_ENDPOINT', 'SHARED_WORLDS_S3_BUCKET', 'SHARED_WORLDS_S3_KEY', 'SHARED_WORLDS_S3_SECRET']
    missing = [n for n in names if not os.environ.get(n)]
    assert not missing, f'no bucket: {missing} (tools/ci/s3_server.sh)'
    return ' '.join(os.environ[n] for n in names)


def spot(player, dy):
    p = player('status')['player']['position']
    return [math.floor(p[0]) + 2, math.floor(p[1]) + dy, math.floor(p[2])]


def run(client, resources, work):
    made = []

    def player(name):
        p = Player(client, resources, work, name)
        made.append(p)
        return p

    try:
        alex, steve = player('Alex'), player('Steve')
        # 1. Alex's world, shared; Steve adds it.
        alex(f'open {WORLD} 0 creative')
        alex.wait('status', lambda v: v['player'] is not None and v['pending_chunks'] == 0, 180, 'the new world')
        alex('disconnect')
        alex.wait('screen', lambda v: v['screen'] == 'title', 120, 'the title after making the world')
        alex(f'shared create {WORLD} {WORLD} {bucket()}')
        task = alex.wait_task(lambda t: not t.startswith('Working'), 300, 'the world shared')['task']
        assert task.startswith('Done'), task
        worlds = alex('shared list')['worlds']
        assert len(worlds) == 1, worlds
        world = worlds[0]['id']
        invite = alex(f'shared invite {world}')['invite']
        assert invite and invite.startswith('HSW1.'), invite
        steve(f'shared add {invite}')
        assert [w['id'] for w in steve('shared list')['worlds']] == [world]
        print('create: shared into the bucket, and added from the invite', flush=True)

        # 2. Nobody plays it: Alex's game takes it and hosts.
        alex(f'shared play {world}')
        hosting = alex.wait_hosting(180, 'Alex hosting')
        print(f'host: Alex hosts (epoch {hosting["epoch"]}, code ready)', flush=True)

        # 3. Steve joins through the lease's code.
        steve(f'shared play {world}')
        steve.wait_joined(180, 'Steve joined Alex')
        alex.wait('shared status', lambda v: 'Steve' in v['session']['players'], 60, 'Steve in the lease')
        first = spot(alex, 6)
        alex(f'command setblock {first[0]} {first[1]} {first[2]} minecraft:diamond_block')
        steve.block(first, 'minecraft:diamond_block', 'the block of Alex on Steve')
        alex.checkpoint('the checkpoint uploaded')
        print('join: Steve joined by the lease\'s code, saw the block; a checkpoint uploaded', flush=True)

        # 4. Alex gives the world to Steve.
        alex(f'shared give {steve.uuid}')
        hosting = steve.wait_hosting(300, 'Steve hosting after the handoff')
        steve.block(first, 'minecraft:diamond_block', 'the block of Alex in the world Steve hosts')
        alex.wait_joined(300, 'Alex joined Steve by itself')
        print(f'give: Steve hosts (epoch {hosting["epoch"]}) with the block, and Alex joined Steve', flush=True)

        # 5. Steve's game dies after a checkpoint: Alex's takes the world from it.
        second = [first[0], first[1] + 2, first[2]]
        steve(f'command setblock {second[0]} {second[1]} {second[2]} minecraft:emerald_block')
        alex.block(second, 'minecraft:emerald_block', 'the block of Steve on Alex')
        steve.checkpoint('the checkpoint of Steve uploaded')
        steve.kill()
        started = time.monotonic()
        hosting = alex.wait_hosting(400, 'Alex taking over')
        print(f'takeover: Alex took the world after {time.monotonic() - started:.0f} s (epoch {hosting["epoch"]})',
              flush=True)
        alex.block(first, 'minecraft:diamond_block', 'the first block after the takeover')
        alex.block(second, 'minecraft:emerald_block', 'the block of the last checkpoint after the takeover')

        # 6. Alex leaves; the last copy goes up and the world closes.
        alex('disconnect')
        task = alex.wait_task(lambda t: t.startswith('Done') or t.startswith('Failed'), 300, 'the last upload')['task']
        assert task.startswith('Done'), task
        chunks = compat.check_regions(alex.home / 'saves' / WORLD)
        print(f'close: the last copy uploaded; {chunks} chunks read back', flush=True)
    except BaseException:
        for p in made:
            print(f'--- {p.name} log tail ---')
            print('\n'.join(p.stop().splitlines()[-60:]))
        raise
    finally:
        for p in made:
            p.stop()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--client', type=Path, required=True)
    parser.add_argument('--resources', type=Path, required=True)
    parser.add_argument('--offscreen', action='store_true')
    parser.add_argument('--xvfb', action='store_true')
    args = parser.parse_args()
    Player.prefix = ['xvfb-run', '-a'] if args.xvfb else []
    Player.extra = ['--offscreen'] if args.offscreen else []
    with tempfile.TemporaryDirectory(prefix='hse-', dir='/tmp') as directory:
        run(args.client.resolve(), args.resources.resolve(), Path(directory))
    print('Shared worlds end to end passed: create, host, join, give, takeover, close')


if __name__ == '__main__':
    main()
