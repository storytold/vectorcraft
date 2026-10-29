#!/usr/bin/env bash
# Have Quick Look make a 512-pixel thumbnail of each document make-test-files.sh wrote, with the
# installed VectorCraft.app's extension (.github/workflows/quicklook-macos.yml), two ways:
# `qlmanage -t -s 512` and examples/ql_probe.rs, which asks QuickLookThumbnailing as Finder does
# (`qlmanage -t` doesn't always reach app-extension thumbnailers on current macOS; a plain PNG is
# run through it first as a control).
#
# Usage: packaging/macos/quicklook/check-thumbnails.sh DOCS_DIR OUT_DIR PROBE
#
# Fails when the probe gets no thumbnail of a good document (or one without the document's
# colours), when it gets one of a bad document, when it gets no answer within $QL_TIMEOUT seconds
# (default 60) on any document, or when qlmanage gets no answer for the control. qlmanage's answers for the documents are recorded, not judged:
# on macOS 15 it gets none for this document type, even for files the extension answers through
# QuickLookThumbnailing in a tenth of a second (the first runs of quicklook-macos.yml; Apple's
# forums report the same, developer.apple.com/forums/thread/784488). Thumbnails go to
# OUT_DIR/thumbnails (qlmanage) and OUT_DIR/probe, qlmanage's output to OUT_DIR/qlmanage.txt; a
# summary is printed (and added to the job summary on GitHub Actions). Space-bar previews are tried
# too (qlmanage -p -o), for the record only.
set -uo pipefail
DOCS="$1"
OUT="$2"
PROBE="$3"
TIMEOUT="${QL_TIMEOUT:-60}"
# qlmanage gets no answer for this type (above): don't wait the full time for it.
QL_WAIT="${QL_WAIT:-20}"
THUMBS="$OUT/thumbnails"
PROBED="$OUT/probe"
mkdir -p "$THUMBS" "$PROBED" "$OUT/previews"
LOG="$OUT/qlmanage.txt"
: >"$LOG"

GOOD=(with-preview.vectorcraft compressed.vectorcraft legacy.drawcraft large.vectorcraft)
BAD=(no-preview.vectorcraft too-new.vectorcraft corrupt.vectorcraft garbage.vectorcraft bomb.vectorcraft)

# Run "$@" for at most $1 seconds of wall-clock time; 124 when it had to be killed.
within() {
  local secs="$1" pid deadline
  shift
  "$@" >>"$LOG" 2>&1 &
  pid=$!
  deadline=$((SECONDS + secs))
  while kill -0 "$pid" 2>/dev/null; do
    if ((SECONDS >= deadline)); then
      kill -9 "$pid" 2>/dev/null
      wait "$pid" 2>/dev/null
      return 124
    fi
    sleep 0.2
  done
  wait "$pid"
}

# Does the PNG $1 have pixels near each "#rrggbb" colour after it? (No colour: "is it a PNG?")
# Python's standard library only: 8- or 16-bit grey, grey and alpha, RGB or RGBA, not interlaced.
# Any other layout fails, so the check can't pass without looking at the pixels.
has_colours() {
  python3 - "$@" <<'PY'
import struct, sys, zlib
path, colours = sys.argv[1], [tuple(int(c[i:i + 2], 16) for i in (1, 3, 5)) for c in sys.argv[2:]]
data = open(path, "rb").read()
if data[:8] != b"\x89PNG\r\n\x1a\n":
    sys.exit("not a PNG")
pos, idat, ihdr = 8, b"", None
while pos + 8 <= len(data):
    n, kind = struct.unpack(">I4s", data[pos:pos + 8])
    body = data[pos + 8:pos + 8 + n]
    if kind == b"IHDR":
        ihdr = struct.unpack(">IIBBBBB", body)
    elif kind == b"IDAT":
        idat += body
    pos += 12 + n
if ihdr is None:
    sys.exit(f"{path}: no IHDR")
w, h, depth, ctype, _, _, interlace = ihdr
channels = {0: 1, 2: 3, 4: 2, 6: 4}.get(ctype)
if depth not in (8, 16) or channels is None or interlace:
    sys.exit(f"{path}: depth {depth}, colour type {ctype}, interlace {interlace}: a layout this check doesn't read")
bpp = channels * depth // 8
raw, stride, rows = zlib.decompress(idat), w * bpp, []
prev = bytearray(stride)
for y in range(h):
    f, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
    for i in range(stride):
        a = line[i - bpp] if i >= bpp else 0
        b, c = prev[i], prev[i - bpp] if i >= bpp else 0
        if f == 1: line[i] = (line[i] + a) & 255
        elif f == 2: line[i] = (line[i] + b) & 255
        elif f == 3: line[i] = (line[i] + (a + b) // 2) & 255
        elif f == 4:
            p = a + b - c
            pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
    rows.append(line)
    prev = line
step = depth // 8  # 16-bit samples: their high byte
def rgb(row, x):
    s = [row[(x * channels + k) * step] for k in range(channels)]
    return tuple(s[:3]) if channels >= 3 else (s[0],) * 3
seen = {rgb(row, x) for row in rows for x in range(w)}
missing = [c for c in colours if not any(sum(abs(p - q) for p, q in zip(c, s)) <= 24 for s in seen)]
if missing:
    sys.exit(f"{path}: no pixels near {['#%02x%02x%02x' % c for c in missing]} ({w}×{h}, {len(seen)} colours)")
print(f"{path}: {w}×{h}, colours found")
PY
}

failed=0
summary="| document | kind | qlmanage -t | probe (QLThumbnailGenerator) | seconds |\n|---|---|---|---|---|\n"

# The control: does qlmanage -t answer for a plain PNG on this machine?
python3 - "$OUT/control.png" <<'PY'
import struct, sys, zlib
w = h = 64
rows = b"".join(b"\0" + bytes((40, 160, 90)) * w for _ in range(h))
def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
open(sys.argv[1], "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
PY
mkdir -p "$OUT/control"
echo "== qlmanage -t -s 512 control.png" >>"$LOG"
within "$TIMEOUT" qlmanage -t -s 512 -o "$OUT/control" "$OUT/control.png"
control=$?
if [[ "$control" = 124 || ! -s "$OUT/control/control.png.png" ]]; then
  echo "qlmanage -t made no thumbnail of a plain PNG (the control, exit $control)"
  failed=1
else
  echo "control: qlmanage -t answered for a plain PNG (exit $control; thumbnail: $(find "$OUT/control" -type f -name '*.png' | tr '\n' ' '))"
fi
summary+="| control.png | control | $([[ $control = 124 ]] && echo "no answer in ${TIMEOUT}s" || echo "exit $control") | | |\n"

check() {
  local kind="$1" name="$2" doc="$DOCS/$2" png="$THUMBS/$2.png" probed="$PROBED/$2.png" start status pstatus ql probe
  start=$SECONDS
  echo "== qlmanage -t -s 512 $name" >>"$LOG"
  within "$QL_WAIT" qlmanage -t -s 512 -o "$THUMBS" "$doc"
  status=$?
  echo "== ql_probe $name" >>"$LOG"
  within "$((TIMEOUT + 5))" "$PROBE" "$doc" "$probed" 512 "$TIMEOUT"
  pstatus=$?
  local took=$((SECONDS - start))
  describe() {
    if [[ -s "$1" ]]; then
      echo "thumbnail $(sips -g pixelWidth -g pixelHeight "$1" 2>/dev/null | awk '/pixel/ {printf "%s ", $2}')"
    elif [[ "$2" = 124 ]]; then
      echo "no answer in ${3}s"
    else
      echo "none (exit $2)"
    fi
  }
  ql="$(describe "$png" "$status" "$QL_WAIT")"
  probe="$(describe "$probed" "$pstatus" "$TIMEOUT")"
  # The probe's own timeout gives 124 too.
  if [[ "$pstatus" = 124 ]]; then
    probe="$probe — HUNG"
    failed=1
  fi
  # Judged by the probe alone (Finder's path): qlmanage's answers are recorded, not judged.
  if [[ "$kind" = good ]]; then
    if [[ "$pstatus" != 0 || ! -s "$probed" ]]; then
      probe="$probe — NO THUMBNAIL"
      failed=1
    elif [[ "$name" = with-preview.vectorcraft ]] && ! has_colours "$probed" "#d0402c" "#2c74d0" >>"$LOG" 2>&1; then
      probe="$probe — WITHOUT the document's colours"
      failed=1
    fi
  elif [[ "$pstatus" = 0 || -s "$probed" ]]; then
    probe="$probe — A THUMBNAIL OF A FILE WITHOUT ONE"
    failed=1
  fi
  echo "$name ($kind): qlmanage $ql; probe $probe; ${took}s"
  summary+="| $name | $kind | $ql | $probe | $took |\n"
  # The Space-bar preview, for the record.
  echo "== qlmanage -p -o $name" >>"$LOG"
  # qlmanage can't host a data-based preview extension (it stops on a fault in
  # ExtensionFoundation: no extension context host class), so this records only that it ends.
  within "$QL_WAIT" qlmanage -p -o "$OUT/previews" "$doc"
  if [[ $? = 124 ]]; then
    echo "$name: qlmanage -p got no answer in ${QL_WAIT}s"
  fi
}

for f in "${GOOD[@]}"; do check good "$f"; done
for f in "${BAD[@]}"; do check bad "$f"; done
ls -lR "$OUT/previews" >>"$LOG" 2>&1
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then printf "%b" "$summary" >>"$GITHUB_STEP_SUMMARY"; fi
printf "%b" "$summary" | tee "$OUT/summary.md"
head -n 400 "$LOG"
exit "$failed"
