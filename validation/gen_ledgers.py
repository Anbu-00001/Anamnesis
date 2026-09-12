import json, datetime, random

def write(claims, path, offsets=None):
    """claims: list of (prob, outcome_bool). offsets: timedelta per claim for the
    resolution timestamp, used to reproduce the resolution-order effect."""
    t0 = datetime.datetime(2025, 1, 1, tzinfo=datetime.timezone.utc)
    out = []
    for i, (p, y) in enumerate(claims):
        created = t0 + datetime.timedelta(hours=i)
        deadline = created + datetime.timedelta(days=30)
        resolved = created + (offsets[i] if offsets else datetime.timedelta(days=31))
        iso = lambda d: d.isoformat().replace("+00:00", "Z")
        out.append(dict(
            id=f"c{i:05d}", statement=f"claim {i}", created_at=iso(created),
            resolve_by=deadline.date().isoformat(), tags=["who:claude"], kind="binary",
            forecasts=[dict(at=iso(created), prob=p)],
            resolution=dict(at=iso(resolved), outcome="true" if y else "false")))
    json.dump({"claims": out}, open(path, "w"))

if __name__ == "__main__":
    import sys
    d = sys.argv[1] if len(sys.argv) > 1 else "."
    rng = random.Random(42)
    write([(0.9 if i % 2 == 0 else 0.1,
            rng.random() < (0.65 if i % 2 == 0 else 0.35)) for i in range(1000)], f"{d}/sym.json")
    mixed = []
    for _ in range(600):
        p = 0.85 if rng.random() < 0.5 else 0.15
        mixed.append((p, rng.random() < (0.65 if p > 0.5 else 0.35)))
    write(mixed, f"{d}/mixed.json")
    write([(p, rng.random() < 0.5 + 0.55 * (p - 0.5))
           for p in (round(rng.uniform(0.6, 0.97), 2) for _ in range(600))], f"{d}/onesided.json")
    print("wrote sym.json mixed.json onesided.json")
