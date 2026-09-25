#!/usr/bin/env bash
# Compare Eratos passes/s between two versions of the code.
#
#   bench.sh [base-ref [new-ref]] [run flags...]
#
# base-ref defaults to HEAD, and new-ref to the working tree. Run flags go to both
# versions and default to all four storage variants. Settings:
#   THREADS  thread counts to test (default "1 all"; "all" means every logical CPU)
#   SECS     seconds per run (default 3)
#   ROUNDS   runs of each version (default 3)
set -euo pipefail

repo=$(git rev-parse --show-toplevel)
base=HEAD
new=""
if [ $# -gt 0 ] && [ "${1#-}" = "$1" ]; then
  base=$1
  shift
fi
if [ $# -gt 0 ] && [ "${1#-}" = "$1" ]; then
  new=$1
  shift
fi
if [ $# -eq 0 ]; then
  set -- --bytes --bits --bits-rotate --bits-striped
fi
for flag in "$@"; do
  case $flag in
    -t* | --threads* | -s* | --seconds*)
      echo "set thread counts with THREADS and run length with SECS, not $flag" >&2
      exit 1
      ;;
  esac
done
threads_list=${THREADS:-1 all}
rounds=${ROUNDS:-3}
secs=${SECS:-3}
for ref in "$base" ${new:+"$new"}; do
  git -C "$repo" rev-parse --verify --quiet "$ref^{commit}" >/dev/null || {
    echo "unknown git ref: $ref" >&2
    exit 1
  }
done
if git -C "$repo" diff --quiet "$base" ${new:+"$new"} -- src Cargo.toml Cargo.lock; then
  echo "note: both sides have the same code, so any change is just noise" >&2
fi

# one work dir per checkout, outside it, and kept between runs so builds can reuse it
work="${TMPDIR:-/tmp}/eratos-bench/$(printf '%s' "$repo" | cksum | cut -d ' ' -f 1)"
mkdir -p "$work"

# binary_name DIR: the binary's name, read from DIR/Cargo.toml as it differs between commits
binary_name() {
  awk -F '"' '/^name = / { print $2; exit }' "$1/Cargo.toml"
}

# build_ref REF NAME: builds a commit without touching the working tree
build_ref() {
  rm -rf "$work/$2-src"
  mkdir -p "$work/$2-src"
  # -m gives the files the current time, so cargo never reuses a build of another commit
  git -C "$repo" archive "$1" | tar -x -m -C "$work/$2-src"
  # commits from before Cargo.lock was tracked don't have one, so use the local one
  if [ ! -f "$work/$2-src/Cargo.lock" ] && [ -f "$repo/Cargo.lock" ]; then
    cp "$repo/Cargo.lock" "$work/$2-src/"
  fi
  cargo build --release --quiet --manifest-path "$work/$2-src/Cargo.toml" --target-dir "$work/$2-target"
  cp "$work/$2-target/release/$(binary_name "$work/$2-src")" "$work/$2"
}

# code_sum BINARY: prints a checksum of its machine code, if there's a tool to read it
code_sum() {
  if command -v otool >/dev/null 2>&1; then
    otool -t "$1" | tail -n +2 | cksum
  elif command -v objcopy >/dev/null 2>&1; then
    objcopy -O binary --only-section=.text "$1" "$work/code.bin" && cksum <"$work/code.bin"
  fi
}

echo "building both versions in release mode..." >&2
build_ref "$base" base
if [ -n "$new" ]; then
  build_ref "$new" new
else
  cargo build --release --quiet --manifest-path "$repo/Cargo.toml" --target-dir "$repo/target"
  # a copy, so a rebuild during the run can't replace it
  cp "$repo/target/release/$(binary_name "$repo")" "$work/new"
fi
same_code=""
base_code=$(code_sum "$work/base" || true)
if [ -n "$base_code" ] && [ "$base_code" = "$(code_sum "$work/new" || true)" ]; then
  same_code=1
fi

# swap which side goes first each round, so slow drift like heat doesn't favour one side
for round in $(seq "$rounds"); do
  if [ $((round % 2)) -eq 1 ]; then sides="base new"; else sides="new base"; fi
  for threads in $threads_list; do
    tflag=""
    if [ "$threads" != all ]; then tflag="-t $threads"; fi
    for side in $sides; do
      echo "round $round/$rounds: $side, $threads thread(s)" >&2
      # Without a terminal there are no colours, so a table row has 7 fields: variant,
      # passes, passes/s, pass time and its unit, primes and a check mark. macOS awk
      # treats ✓ and ✗ as equal when compared with ==, so index() checks them instead.
      # shellcheck disable=SC2086
      rows=$("$work/$side" -s "$secs" $tflag "$@" |
        awk -v side="$side" -v threads="$threads" '
          NF == 7 && $1 ~ /storage/ {
            gsub(",", "", $3)
            if (index($7, "✓")) check = "ok"
            else if (index($7, "✗")) check = "wrong"
            else check = "unchecked"
            print side, threads, $1, $3, check
          }')
      if [ -z "$rows" ]; then
        echo "no result rows from the $side build; commits before 71030eb print a different format" >&2
        exit 1
      fi
      echo "$rows"
    done
  done
done >"$work/results"

new_label="working tree"
if [ -n "$new" ]; then new_label="$new ($(git -C "$repo" rev-parse --short "$new"))"; fi
echo
echo "base = $base ($(git -C "$repo" rev-parse --short "$base")), new = $new_label;" \
  "best of $rounds runs of ${secs} s each, in passes/s"
if [ -n "$same_code" ]; then
  echo "note: both builds have the same machine code, so any change below is just noise"
fi
awk '
  {
    key = $2 " " $3
    if (!(key in seen)) { seen[key] = 1; keys[++n] = key }
    k = $1 " " key
    runs[k]++; total[k] += $4
    if (!(k in best) || $4 > best[k]) best[k] = $4
    if (!(k in worst) || $4 < worst[k]) worst[k] = $4
    if ($5 != "ok") bad = bad "\n  " $1 ", " $2 " thread(s), " $3 ": " $5
  }
  function pct(new, old) { return (new / old - 1) * 100 }
  function spread(k) { return (best[k] - worst[k]) / (total[k] / runs[k]) * 100 }
  END {
    printf "%-8s%-21s%10s%10s%9s%10s%8s\n", "threads", "variant", "base", "new", "change", "mean chg", "spread"
    for (i = 1; i <= n; i++) {
      split(keys[i], f, " ")
      b = "base " keys[i]; w = "new " keys[i]
      if (!(b in runs) || !(w in runs)) {
        printf "%-8s%-21s  only in %s\n", f[1], f[2], (b in runs) ? "base" : "new"
        continue
      }
      # one run per side says nothing about noise
      s = "-"
      if (runs[b] > 1 && runs[w] > 1) {
        s = spread(b); if (spread(w) > s) s = spread(w)
        s = sprintf("%.1f%%", s)
      }
      printf "%-8s%-21s%10d%10d%+8.1f%%%+9.1f%%%8s\n", f[1], f[2], best[b], best[w],
        pct(best[w], best[b]), pct(total[w] / runs[w], total[b] / runs[b]), s
    }
    if (bad != "") print "\nwrong or unchecked prime counts:" bad
  }
' "$work/results"
