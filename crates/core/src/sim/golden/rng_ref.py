import struct

M = 2**64 - 1
GAMMA = 0x9E3779B97F4A7C15
FORK_TAG = 0xD1B54A32D192ED03


def mix64(v):
    z = (v + GAMMA) & M
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M
    return z ^ (z >> 31)


def rotl(x, k):
    return ((x << k) | (x >> (64 - k))) & M


class Rng:
    def __init__(self, seed):
        self.root = seed
        sm = seed
        self.s = []
        for _ in range(4):
            z = (sm + GAMMA) & M
            z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M
            z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M
            self.s.append(z ^ (z >> 31))
            sm = (sm + GAMMA) & M

    def fork(self, label):
        return Rng(mix64((mix64(self.root ^ FORK_TAG) + mix64(label)) & M))

    def u64(self):
        s = self.s
        result = (rotl((s[1] * 5) & M, 7) * 9) & M
        t = (s[1] << 17) & M
        s[2] ^= s[0]
        s[3] ^= s[1]
        s[1] ^= s[2]
        s[0] ^= s[3]
        s[2] ^= t
        s[3] = rotl(s[3], 45)
        return result

    def u32(self):
        return self.u64() >> 32

    def f64(self):
        return (self.u64() >> 11) * 2.0**-53

    def f32(self):
        return (self.u64() >> 40) * 2.0**-24

    def below(self, n):
        m = self.u64() * n
        low = m & M
        if low < n:
            t = ((-n) & M) % n
            while low < t:
                m = self.u64() * n
                low = m & M
        return m >> 64

    def shuffle(self, items):
        for i in range(len(items) - 1, 0, -1):
            j = self.below(i + 1)
            items[i], items[j] = items[j], items[i]

    def weighted_u32(self, weights):
        total = sum(weights)
        target = self.below(total)
        for i, w in enumerate(weights):
            if target < w:
                return i
            target -= w

    def weighted_f64(self, weights):
        total = 0.0
        for w in weights:
            total += w
        target = self.f64() * total
        acc = 0.0
        last = None
        for i, w in enumerate(weights):
            if w > 0.0:
                last = i
                acc += w
                if target < acc:
                    return i
        return last


def fbits(f):
    return "%016x" % struct.unpack("<Q", struct.pack("<d", f))[0]


def f32bits(f):
    return "%08x" % struct.unpack("<I", struct.pack("<f", f))[0]


def line(head, values):
    print(head, ":", " ".join(values))


def save_bytes(r):
    out = struct.pack("<IQ", 1, r.root) + b"".join(struct.pack("<Q", w) for w in r.s)
    return out.hex()


def main():
    for seed in (0, 42, 0xDEADBEEFCAFEF00D):
        r = Rng(seed)
        line("u64 %d" % seed, ["%016x" % r.u64() for _ in range(1000)])
    r = Rng(42)
    line("u32 42", ["%08x" % r.u32() for _ in range(100)])
    r = Rng(7)
    line("f64 7", [fbits(r.f64()) for _ in range(100)])
    r = Rng(7)
    line("f32 7", [f32bits(r.f32()) for _ in range(100)])
    for n in (6, 1000, 10000000000000000000, 3 * 2**62):
        r = Rng(7)
        line("below 7 %d" % n, [str(r.below(n)) for _ in range(100)])
    r = Rng(7)
    line("range_i64 7 -5 5", [str(-5 + r.below(10)) for _ in range(100)])
    for seed, n in ((7, 20), (8, 52)):
        r = Rng(seed)
        items = list(range(n))
        r.shuffle(items)
        line("shuffle %d %d" % (seed, n), [str(i) for i in items])
    w = [1, 2, 3, 0, 10]
    r = Rng(7)
    line("weighted_u32 7 1,2,3,0,10", [str(r.weighted_u32(w)) for _ in range(100)])
    w = [0.5, 0.0, 2.5, 1.0]
    r = Rng(7)
    line("weighted_f64 7 0.5,0,2.5,1", [str(r.weighted_f64(w)) for _ in range(100)])
    for label in (1, 2):
        r = Rng(7).fork(label)
        line("fork 7 %d" % label, ["%016x" % r.u64() for _ in range(50)])
    r = Rng(7).fork(1).fork(2)
    line("fork2 7 1 2", ["%016x" % r.u64() for _ in range(50)])
    for seed in (0, 42, 0xDEADBEEFCAFEF00D):
        for draws in (0, 1, 1000):
            r = Rng(seed)
            for _ in range(draws):
                r.u64()
            line("save %d %d" % (seed, draws), [save_bytes(r)])
    r = Rng(7).fork(1)
    line("save_fork 7 1 0", [save_bytes(r)])
    for _ in range(100):
        r.u64()
    line("save_fork 7 1 100", [save_bytes(r)])


main()
