#!/usr/bin/env bash
# Make the documents the Quick Look workflow (.github/workflows/quicklook-macos.yml) checks, with
# vectorcraft-cli. Written from scratch: no file here comes from another application.
#
# Usage: packaging/macos/quicklook/make-test-files.sh CLI OUT_DIR
#
# Good files (each must get a thumbnail):
#   with-preview.vectorcraft  a new file, saved with the default Embed Preview
#   compressed.vectorcraft    gzipped, version 4, with an image kept after the JSON
#   legacy.drawcraft          the former name's extension and format name (version 1)
#   large.vectorcraft         50,000 paths
# Files without a thumbnail of their own (Finder shows the icon; none may hang):
#   no-preview.vectorcraft    saved with preview off
#   too-new.vectorcraft       a version newer than this VectorCraft reads
#   corrupt.vectorcraft       a compressed file cut short
#   garbage.vectorcraft       random bytes
#   bomb.vectorcraft          gzip that unpacks past the format's MAX_JSON
set -euo pipefail
CLI="$1"
OUT="$2"
mkdir -p "$OUT"

# Two coloured shapes on a 600 × 400 artboard, saved with the native options in $1 (JSON members).
save() {
  local path="$1" options="$2"
  "$CLI" run --cmd file.new --params '{"width":600,"height":400}' \
    --cmd shape.rectangle --params '{"x":40,"y":40,"width":320,"height":220}' \
    --cmd paint.setFill --params '{"color":"#d0402c"}' \
    --cmd shape.ellipse --params '{"x":220,"y":150,"width":330,"height":220}' \
    --cmd paint.setFill --params '{"color":"#2c74d0"}' \
    --cmd document.save --params "{\"path\":\"$path\"${options:+,$options}}" >/dev/null
}

save "$OUT/with-preview.vectorcraft" ""
save "$OUT/no-preview.vectorcraft" '"preview":false'
save "$OUT/legacy.drawcraft" '"version":1'

# Compressed, with a placed image large enough to go after the JSON (version 4).
python3 - "$OUT/noise.png" <<'PY'
import random, struct, sys, zlib
w = h = 96
random.seed(4)
rows = b"".join(b"\0" + bytes(random.randrange(256) for _ in range(w * 3)) for _ in range(h))
def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
PY
"$CLI" run --cmd file.new --params '{"width":600,"height":400}' \
  --cmd file.place --params "{\"path\":\"$OUT/noise.png\",\"link\":false}" \
  --cmd shape.ellipse --params '{"x":300,"y":100,"width":250,"height":250}' \
  --cmd paint.setFill --params '{"color":"#3aa14a"}' \
  --cmd document.save --params "{\"path\":\"$OUT/compressed.vectorcraft\",\"compress\":true,\"version\":4}" >/dev/null
rm -f "$OUT/noise.png"

# 50,000 small paths, drawn as an SVG and saved as a native file.
python3 - "$OUT/large.svg" <<'PY'
import random, sys
random.seed(50)
out = ['<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="1200" viewBox="0 0 1600 1200">']
for i in range(50_000):
    x, y = random.uniform(0, 1580), random.uniform(0, 1180)
    out.append(f'<path d="M{x:.1f} {y:.1f}l{random.uniform(4, 20):.1f} {random.uniform(-8, 8):.1f}l{random.uniform(-8, 8):.1f} {random.uniform(4, 20):.1f}z" fill="#{random.randrange(0x1000000):06x}"/>')
out.append("</svg>")
open(sys.argv[1], "w").write("\n".join(out))
PY
"$CLI" run --in "$OUT/large.svg" --cmd document.save --params "{\"path\":\"$OUT/large.vectorcraft\"}" >/dev/null
rm -f "$OUT/large.svg"

python3 - "$OUT" <<'PY'
import gzip, json, os, random, sys
out = sys.argv[1]
# A version from the future: everything else as a good file.
f = json.load(open(os.path.join(out, "with-preview.vectorcraft")))
f["version"] = 99
json.dump(f, open(os.path.join(out, "too-new.vectorcraft"), "w"))
# A compressed file cut short.
whole = open(os.path.join(out, "compressed.vectorcraft"), "rb").read()
open(os.path.join(out, "corrupt.vectorcraft"), "wb").write(whole[: len(whole) // 3])
random.seed(7)
open(os.path.join(out, "garbage.vectorcraft"), "wb").write(bytes(random.randrange(256) for _ in range(100_000)))
# 1.1 GB of a JSON string, gzipped to about 1 MB: past MAX_JSON (1 GiB).
with gzip.open(os.path.join(out, "bomb.vectorcraft"), "wb", compresslevel=9) as g:
    g.write(b'{"format":"vectorcraft","version":4,"preview":null,"pad":"')
    chunk = b"a" * (1 << 20)
    for _ in range(1100):
        g.write(chunk)
    g.write(b'"}')
PY
ls -l "$OUT"
