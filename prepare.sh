#!/bin/sh
# Salamander Grand Piano V3 完全版（24 ビット FLAC）から、C4 の比較に使う
# サンプルを salamander/ に WAV でデコードする。要 flac コマンド。
set -e
SRC="${1:-../external/salamander-sfz/extracted/SalamanderGrandPiano-SFZ+FLAC-V3+20200602/samples}"
DST="$(dirname "$0")/salamander"
mkdir -p "$DST"
dec() { flac -d -f -s -o "$DST/$2.wav" "$SRC/$1.flac"; echo "$1 -> $DST/$2.wav"; }
for v in 4 8 14; do dec "C4v$v" "C4v$v"; done   # 打弦（ベロシティ層 4, 8, 14）
dec rel40 C4_release                               # 離鍵（ダンパーが弦に触れる音、鍵 60）
dec harmLC4 C4_pedal_resonance_loud                # ペダルを踏んで離鍵したときの弦共鳴（強）
dec harmSC4 C4_pedal_resonance_soft                # 同上（弱）
dec pedalD1 pedal_down                             # ペダルを踏む音
dec pedalU1 pedal_up                               # ペダルを離す音
