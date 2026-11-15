#!/usr/bin/env bash
# Keeps what the jobs build, as Nix binary caches in directories, which Nix
# reads back as substituters.  What is kept is what the public binary cache
# does not have: what a job built or downloaded itself.
#
#   store.sh new DIR INSTALLABLE...
#       Copies to DIR everything of that kind in the build closure of the
#       installables that is in the job's store: their outputs, and what
#       building them needed (sources, dependency layers, toolchains).
#       Paths that a directory listed in $KNOWN has already are left out,
#       and so is what the new paths refer to that a directory listed in
#       $KEPT has.
#
#   store.sh results DIR OUT-LINK...
#       Copies to DIR only the built results and what they refer to.
#
#   store.sh prune DIR INSTALLABLE...
#       Removes from DIR what is not in the build closure of the
#       installables, and what the public cache has.  Prints a name for
#       what is left.
set -euo pipefail
export LC_ALL=C

mode=$1
dir=$2
shift 2

# The build closure of the installables, as store paths: every output of
# every derivation, built or not, and every source.
closure() {
  # shellcheck disable=SC2046 # one argument per derivation
  nix-store --query --requisites $(nix path-info --derivation "$@") > "$tmp/closure"
  {
    grep --invert-match '\.drv$' "$tmp/closure" || true
    grep '\.drv$' "$tmp/closure" | xargs --no-run-if-empty nix-store --query --outputs
  } | sort --unique
}

# Of the sorted store paths on standard input, those that are in the store
# and do not carry the public cache's signature.
ours() {
  nix path-info --all --sigs |
    { grep --invert-match 'cache\.nixos\.org-1:' || true; } |
    cut --fields=1 | tr --delete ' ' | sort > "$tmp/ours"
  comm -12 - "$tmp/ours"
}

# Of the store paths on standard input, those that no directory of $KNOWN
# has.
unknown() {
  while read -r path; do
    hash=${path#/nix/store/}
    hash=${hash%%-*}
    for known in ${KNOWN:-}; do
      if [ -e "$known/$hash.narinfo" ]; then continue 2; fi
    done
    echo "$path"
  done
}

# A binary cache takes a path only together with the paths it refers to, so
# those are copied too, whichever cache they came from.
copy() {
  xargs --no-run-if-empty nix copy --to "file://$dir?compression=zstd"
}

# The paths that DIR has, as hashes: /nix/store/<hash>-<name> is the file
# <hash>.narinfo, which names the path's archive under nar/.
paths() {
  (cd "$dir" && ls) | sed --quiet 's/\.narinfo$//p' | sort
}

# Of them, those of the public cache.
public() {
  { find "$dir" -maxdepth 1 -name '*.narinfo' -exec grep --files-with-matches '^Sig: cache\.nixos\.org-1:' {} + || true; } |
    sed 's|.*/||; s/\.narinfo$//'
}

# Removes the paths on standard input from DIR, and then the archives that
# no path names any longer; paths with the same contents share one.
remove() {
  while read -r hash; do
    rm "$dir/$hash.narinfo"
  done
  mkdir --parents "$dir/nar"
  find "$dir" -maxdepth 1 -name '*.narinfo' -exec sed --quiet 's/^URL: //p' {} + | sort --unique > "$tmp/archives"
  (cd "$dir" && find nar -type f) | sort | comm -23 - "$tmp/archives" | while read -r archive; do
    rm "$dir/$archive"
  done
}

tmp=$(mktemp --directory)
mkdir --parents "$dir"

case $mode in
  new)
    closure "$@" | ours | unknown | copy
    # What came along only because a new path refers to it: paths of the
    # public cache, and paths the directories listed in $KEPT have (the
    # cache that every later job restores too).
    {
      public
      for kept in ${KEPT:-}; do
        (cd "$kept" && ls) | sed --quiet 's/\.narinfo$//p'
      done
    } | sort --unique > "$tmp/known"
    paths | comm -12 - "$tmp/known" | remove
    ;;
  results)
    nix-store --query --requisites "$@" | sort | ours | copy
    ;;
  prune)
    closure "$@" | cut --characters=12-43 | sort --unique > "$tmp/wanted"
    {
      paths | comm -23 - "$tmp/wanted"
      public
    } | sort --unique | remove
    paths | sha256sum | cut --characters=1-32
    ;;
  *)
    echo "usage: $0 new|results|prune DIR ARGUMENT..." >&2
    exit 2
    ;;
esac
