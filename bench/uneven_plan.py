"""Portable exact seeded path plan used by the final uneven fixture.
No files are created by this module; each tuple is(kind,relativepath).
"""
import random
SEED=2026111051

def plan():
    rng = random.Random(SEED); items = []
    def directory(path): items.append(('directory', path))
    def files(path, count):
        for number in range(count):
            prefix = f'f{number:05}-{rng.getrandbits(32):08x}'
            tail = '-'+('x' * rng.randrange(64, 177)) if number % 5 == 0 else ''
            items.append(('file', path + '/' + prefix + tail))
    for group in range(64):
        current = f'g{group:03}'; directory(current)
        for depth in range(rng.randrange(1, 10)):
            for sibling in range(rng.randrange(5)):
                side = current + f'/side{depth}-{sibling}'; directory(side)
                files(side, rng.randrange(4))
            current += f'/branch{depth}'; directory(current)
        files(current, rng.choice((0, 1, 8, 64, 512, 1024)))
    directory('heavy')
    for branch in range(8):
        path = f'heavy/family{branch}'; directory(path); files(path, 512)
    assert len({name for _, name in items}) == len(items)
    assert len(items) + 3 <= 80000
    return items
