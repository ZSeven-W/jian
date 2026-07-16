#!/usr/bin/env bash
set -euo pipefail

player_dir="$(cd "$(dirname "$0")/.." && pwd)"
repo_dir="$(cd "$player_dir/../.." && pwd)"
header_dir="$repo_dir/crates/jian-engine-ffi/include"
fixture="$repo_dir/crates/jian-core/tests/fixtures/m1_acceptance.json"

required=(
  "$player_dir/JianPlayer-Bridging-Header.h"
  "$player_dir/project.yml"
  "$player_dir/README.md"
  "$player_dir/Resources/m1_acceptance.op"
  "$player_dir/Sources/JianPlayerApp.swift"
  "$player_dir/Sources/JianPlayerView.swift"
  "$player_dir/Sources/JianEngineHost.swift"
  "$player_dir/Sources/JianTextInput.swift"
  "$player_dir/Sources/JianCapabilities.swift"
)

for path in "${required[@]}"; do
  if [[ ! -f "$path" ]]; then
    echo "missing required iOS Player source: $path" >&2
    exit 1
  fi
done

cmp "$fixture" "$player_dir/Resources/m1_acceptance.op"

while IFS= read -r source; do
  lines="$(wc -l < "$source" | tr -d ' ')"
  if (( lines > 800 )); then
    echo "$source has $lines lines; new files are capped at 800" >&2
    exit 1
  fi
done < <(find "$player_dir/Sources" -name '*.swift' -type f -print)

ruby - "$player_dir/project.yml" <<'RUBY'
require "yaml"
project = YAML.safe_load(File.read(ARGV.fetch(0)), aliases: true)
target = project.fetch("targets").fetch("JianPlayer")
raise "JianPlayer must be an iOS application" unless target["platform"] == "iOS" && target["type"] == "application"
settings = target.fetch("settings").fetch("base")
raise "deployment target must be iOS 15+" unless settings.fetch("IPHONEOS_DEPLOYMENT_TARGET").to_f >= 15.0
raise "bridging header setting missing" unless settings.key?("SWIFT_OBJC_BRIDGING_HEADER")
raise "jian.h search path missing" unless settings.fetch("HEADER_SEARCH_PATHS").to_s.include?("jian-engine-ffi/include")
raise "device staticlib search path missing" unless settings.fetch("LIBRARY_SEARCH_PATHS[sdk=iphoneos*]").include?("aarch64-apple-ios/release")
raise "simulator staticlib search path missing" unless settings.fetch("LIBRARY_SEARCH_PATHS[sdk=iphonesimulator*]").include?("aarch64-apple-ios-sim/release")
frameworks = target.fetch("dependencies").map { |entry| entry["sdk"] }.compact
%w[Metal.framework QuartzCore.framework UIKit.framework].each do |framework|
  raise "#{framework} dependency missing" unless frameworks.include?(framework)
end
RUBY

grep -Fq -- "-sdk iphonesimulator26.4" "$player_dir/README.md"
grep -Fq -- "-destination 'platform=iOS Simulator,id=<sim-id>'" "$player_dir/README.md"
grep -Fq -- "LIBRARY_SEARCH_PATHS=/Users/kayshen/Workspace/ZSeven-W/jian/target/aarch64-apple-ios-sim/release" "$player_dir/README.md"
grep -Fq -- "-ljian_engine_ffi -lc++" "$player_dir/README.md"
grep -Fq -- "-framework Metal" "$player_dir/README.md"

sdk="$(xcrun --sdk iphonesimulator --show-sdk-path)"
target="arm64-apple-ios15.0-simulator"
module_cache="${TMPDIR:-/tmp}/jian-ios-player-module-cache"
mkdir -p "$module_cache"
export CLANG_MODULE_CACHE_PATH="$module_cache"

xcrun clang \
  -target "$target" \
  -isysroot "$sdk" \
  -fsyntax-only \
  -x objective-c \
  -I "$header_dir" \
  "$player_dir/JianPlayer-Bridging-Header.h"

xcrun swiftc \
  -typecheck \
  -warnings-as-errors \
  -parse-as-library \
  -target "$target" \
  -sdk "$sdk" \
  -import-objc-header "$player_dir/JianPlayer-Bridging-Header.h" \
  -module-cache-path "$module_cache" \
  -Xcc -I \
  -Xcc "$header_dir" \
  "$player_dir"/Sources/*.swift

echo "iOS Player sources and ABI imports validate"
