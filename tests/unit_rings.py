"""Binary64 unit-circle inputs shared by geometry parity tests."""
import gzip
import struct
from pathlib import Path
packed = (Path(__file__).parent / "fixtures/earcut_unit_rings.f64.gz").read_bytes()
pairs = struct.iter_unpack("<dd", gzip.decompress(packed))
rings = {size: [next(pairs) for _ in range(size)] for size in range(3, 65)}
