#!/bin/sh
# Downloads the converter's libraries (deps.txt) into lib/ and checks each one: against Maven
# Central's SHA-1 the first time, then against the SHA-256 recorded in deps.lock, so a
# changed file is noticed.
set -eu
cd "$(dirname "$0")"
mkdir -p lib
REPO=https://repo1.maven.org/maven2
touch deps.lock
grep -v '^#' deps.txt | grep . | while IFS=: read -r group artifact version; do
  jar="$artifact-$version.jar"
  url="$REPO/$(echo "$group" | tr . /)/$artifact/$version/$jar"
  if [ ! -f "lib/$jar" ]; then
    curl -sfL "$url" -o "lib/$jar.part"
    want=$(curl -sfL "$url.sha1" | cut -c1-40)
    got=$(shasum -a 1 "lib/$jar.part" | cut -c1-40)
    if [ "$want" != "$got" ]; then echo "checksum mismatch for $jar" >&2; rm -f "lib/$jar.part"; exit 1; fi
    mv "lib/$jar.part" "lib/$jar"
  fi
  sum=$(shasum -a 256 "lib/$jar" | cut -c1-64)
  recorded=$(grep " $jar\$" deps.lock | cut -c1-64 || true)
  if [ -z "$recorded" ]; then echo "$sum  $jar" >> deps.lock
  elif [ "$recorded" != "$sum" ]; then echo "$jar differs from deps.lock" >&2; exit 1; fi
done
echo "libraries ready in $(pwd)/lib"
