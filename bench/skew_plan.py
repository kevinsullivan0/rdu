"""Portable original skew path plan; no filesystem access or creation."""
import random
SEED = 2026111103


def plan():
    rng = random.Random(SEED)
    # Original generator created burst first:128 leaves x256 random file names.
    for _ in range(128 * 256):
        rng.getrandbits(32)
    items = [('directory', f'empty{number:05}') for number in range(4000)]
    for branch in range(4):
        root = f'hot{branch}'
        items.append(('directory', root))
        for number in range(15000):
            items.append(('file', root + f'/f{number:05}-{rng.getrandbits(32):08x}'))
    return items
