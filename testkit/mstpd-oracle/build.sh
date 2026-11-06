#!/bin/sh
# Builds the mstpd oracle: fetches mstpd at a pinned commit and links its protocol core with
# harness.c. Prints the path of the binary. Works on Linux and macOS (macOS uses compat/darwin for
# the few Linux headers mstp.c includes).
#
#   MSTPD_ORACLE=$(testkit/mstpd-oracle/build.sh) cargo test -p netdev --test stp_oracle
set -eu

# mstpd master as of 2026-09-18. Change deliberately: a new mstpd can change the answers.
MSTPD_REPO=https://github.com/mstpd/mstpd.git
MSTPD_REV=2e747d80ad48ffd32ec2f918b48a7c7a3916253b

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
out="$root/target/mstpd-oracle"
src="$out/mstpd-$MSTPD_REV"
bin="$out/mstpd-oracle"

mkdir -p "$out"
if [ ! -f "$src/mstp.c" ]; then
    rm -rf "$src.tmp"
    git init -q "$src.tmp"
    git -C "$src.tmp" fetch -q --depth 1 "$MSTPD_REPO" "$MSTPD_REV"
    git -C "$src.tmp" checkout -q FETCH_HEAD
    mv "$src.tmp" "$src"
fi

inc="-I$here/compat -I$src"
if [ "$(uname -s)" = Darwin ]; then
    inc="-I$here/compat/darwin $inc"
fi

if [ ! -x "$bin" ] || [ "$here/harness.c" -nt "$bin" ] || [ "$0" -nt "$bin" ]; then
    # -w: mstp.c is not ours to fix; harness.c is checked with the warnings below.
    ${CC:-cc} -O1 -std=gnu99 -DNO_DAEMON $inc -w -c "$src/mstp.c" -o "$out/mstp.o"
    ${CC:-cc} -O1 -std=gnu99 -DNO_DAEMON $inc -w -c "$src/hmac_md5.c" -o "$out/hmac_md5.o"
    ${CC:-cc} -O1 -std=gnu99 -DNO_DAEMON $inc -Wall -Werror -Wno-unused-function \
        -c "$here/harness.c" -o "$out/harness.o"
    ${CC:-cc} -o "$bin" "$out/harness.o" "$out/mstp.o" "$out/hmac_md5.o"
fi
echo "$bin"
