#!/bin/sh
# Builds the .mpp converter into out/: the converter and its libraries (out/lib) and a Java
# runtime holding only the modules they use (out/runtime), made with jlink. Super PM runs
#   out/runtime/bin/java -cp "out/lib/*" MppToXml input.mpp output.xml
# Needs a JDK 21 (JAVA_HOME, or Homebrew's openjdk@21) and the libraries (./fetch.sh).
set -eu
# Homebrew Java prints a notice for this variable on every run.
unset JAVA_TOOL_OPTIONS
cd "$(dirname "$0")"
JAVA_HOME=${JAVA_HOME:-/opt/homebrew/opt/openjdk@21}
[ -x "$JAVA_HOME/bin/jlink" ] || { echo "no JDK 21 at $JAVA_HOME; install one (brew install openjdk@21) or set JAVA_HOME" >&2; exit 1; }
./fetch.sh >/dev/null
rm -rf build out
mkdir -p build/classes out/lib
"$JAVA_HOME/bin/javac" --release 21 -nowarn -cp "lib/*" -d build/classes src/MppToXml.java
"$JAVA_HOME/bin/jar" --create --file out/lib/mpp-to-xml.jar -C build/classes .
cp lib/*.jar out/lib/
# The Java modules the converter and its libraries need, plus jdk.charsets for old files'
# text encodings and jdk.zipfs for zipped data.
modules=$("$JAVA_HOME/bin/jdeps" --ignore-missing-deps --multi-release 21 --print-module-deps -q --module-path lib --add-modules ALL-MODULE-PATH)
"$JAVA_HOME/bin/jlink" --add-modules "$modules,jdk.charsets,jdk.zipfs" --strip-debug --no-man-pages --no-header-files \
  --compress=zip-6 --output out/runtime
cp -R THIRD-PARTY.md licenses out/
echo "built $(pwd)/out ($(du -sh out | cut -f1)); modules: $modules"
