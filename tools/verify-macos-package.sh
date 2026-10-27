#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
repo_root=${script_dir:h}
package_root="$repo_root/target/package/macos"
app_bundle=${1:-"$package_root/Low Poly Earth.app"}
dmg_path=${2:-"$package_root/Low Poly Earth-Apple-Silicon.dmg"}
executable="$app_bundle/Contents/MacOS/Low Poly Earth"
plist="$app_bundle/Contents/Info.plist"
icon="$app_bundle/Contents/Resources/AppIcon.icns"
resources="$app_bundle/Contents/Resources"
project_license="$resources/LICENSE"
liberty_license="$resources/LICENSE-StatueOfLiberty-MIT.txt"
ofl_plex="$resources/OFL-IBMPlex.txt"
ofl_noto="$resources/OFL-NotoSansCJK.txt"
notices="$resources/THIRD_PARTY_NOTICES.md"
privacy_policy="$resources/PrivacyPolicy.html"
privacy_manifest="$resources/PrivacyInfo.xcprivacy"

for required in \
    "$app_bundle" "$executable" "$plist" "$icon" \
    "$project_license" "$liberty_license" "$ofl_plex" "$ofl_noto" "$notices" "$privacy_policy" "$privacy_manifest"; do
    if [[ ! -e "$required" ]]; then
        print -u2 "missing package component: $required"
        exit 1
    fi
done

cmp "$repo_root/LICENSE" "$project_license"
cmp "$repo_root/crates/vearth-gen/assets/landmarks/statue_of_liberty-LICENSE.txt" "$liberty_license"
cmp "$repo_root/crates/vearth-app/assets/fonts/OFL.txt" "$ofl_plex"
cmp "$repo_root/crates/vearth-app/assets/fonts/OFL-NotoSansCJK.txt" "$ofl_noto"
python3 "$repo_root/tools/verify-title-fonts.py"
for title_license in "$repo_root"/crates/vearth-app/assets/fonts/titles/*-OFL.txt; do
    cmp "$title_license" "$resources/TitleFontLicenses/${title_license:t}"
done
expected_notices=$(mktemp "${TMPDIR:-/tmp}/vearth-third-party-notices.XXXXXX")
trap 'rm -f "$expected_notices"' EXIT
"$repo_root/tools/write-third-party-notices.sh" "$expected_notices"
if ! cmp "$expected_notices" "$notices"; then
    print -u2 "THIRD_PARTY_NOTICES.md does not match the source manifests"
    exit 1
fi

python3 "$repo_root/tools/write-privacy-policy.py" "$expected_notices"
cmp "$expected_notices" "$privacy_policy"
cmp "$repo_root/packaging/macos/PrivacyInfo.xcprivacy" "$privacy_manifest"
plutil -lint "$privacy_manifest"
expected_copyright=$(plutil -extract NSHumanReadableCopyright raw "$repo_root/packaging/macos/Info.plist")
actual_copyright=$(plutil -extract NSHumanReadableCopyright raw "$plist")
if [[ -z "$actual_copyright" || "$actual_copyright" != "$expected_copyright" ]]; then
    print -u2 "missing or mismatched application copyright"
    exit 1
fi

plutil -lint "$plist"
bundle_id=$(plutil -extract CFBundleIdentifier raw "$plist")
bundle_version=$(plutil -extract CFBundleShortVersionString raw "$plist")
minimum_macos=$(plutil -extract LSMinimumSystemVersion raw "$plist")
expected_minimum_macos=$(plutil -extract LSMinimumSystemVersion raw "$repo_root/packaging/macos/Info.plist")
expected_version=$(awk '
    $0 == "[workspace.package]" { workspace = 1; next }
    workspace && $0 ~ /^\[/ { exit }
    workspace && $1 == "version" {
        gsub(/"/, "", $3)
        print $3
        exit
    }
' "$repo_root/Cargo.toml")
if [[ "$bundle_id" != com.blueblazin.voxelearth ]]; then
    print -u2 "unexpected bundle identifier: $bundle_id"
    exit 1
fi
for name_key in CFBundleDisplayName CFBundleName CFBundleExecutable; do
    name_value=$(plutil -extract "$name_key" raw "$plist")
    if [[ "$name_value" != "Low Poly Earth" ]]; then
        print -u2 "unexpected $name_key: $name_value"
        exit 1
    fi
done
if [[ -z "$expected_version" || "$bundle_version" != "$expected_version" ]]; then
    print -u2 "application version $bundle_version does not match workspace $expected_version"
    exit 1
fi
if [[ "$minimum_macos" != "$expected_minimum_macos" ]]; then
    print -u2 "application minimum macOS $minimum_macos does not match template $expected_minimum_macos"
    exit 1
fi
if [[ "$(file "$executable")" != *"Mach-O 64-bit executable arm64"* ]]; then
    print -u2 "application executable is not arm64"
    exit 1
fi
if [[ "$(file "$icon")" != *"Mac OS X icon"* ]]; then
    print -u2 "AppIcon.icns is not a valid macOS icon"
    exit 1
fi
if otool -L "$executable" |
    grep -E '/opt/homebrew|/usr/local|@rpath|@loader_path' >/dev/null; then
    print -u2 "application has a non-system dynamic-library dependency"
    otool -L "$executable"
    exit 1
fi

linked_minimum=$(otool -l "$executable" |
    awk '$1 == "minos" { print $2; exit }')
if [[ -z "$linked_minimum" ]]; then
    print -u2 "could not read the executable minimum macOS version"
    exit 1
fi
if [[ "$linked_minimum" != "$minimum_macos" ]]; then
    print -u2 "executable minimum macOS $linked_minimum does not match Info.plist $minimum_macos"
    exit 1
fi

print "Application bundle checks passed"
print "  identifier: $bundle_id"
print "  version: $bundle_version"
print "  architecture: arm64"
print "  Info.plist minimum macOS: $minimum_macos"
print "  Mach-O minimum macOS: $linked_minimum"
signing_authority=$(codesign --display --verbose=2 "$app_bundle" 2>&1 |
    awk -F= '$1 == "Authority" { print $2; exit }' || true)
if [[ "$signing_authority" == "Developer ID Application:"* ]]; then
    codesign --verify --strict --deep --verbose=2 "$app_bundle"
    if ! codesign --display --verbose=2 "$app_bundle" 2>&1 | grep -E '^CodeDirectory .*flags=.*runtime' >/dev/null; then
        print -u2 "Developer ID signature lacks the hardened runtime"
        exit 1
    fi
    print "  signing: $signing_authority (hardened runtime)"
else
    print "  signing: ${signing_authority:-unsigned} (not for distribution)"
fi

if [[ -f "$dmg_path" ]]; then
    hdiutil verify "$dmg_path"
    if xcrun stapler validate "$dmg_path" >/dev/null 2>&1; then
        spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg_path"
        print "DMG notarization ticket stapled and accepted by Gatekeeper"
    else
        print "DMG is not notarized (not for distribution)"
    fi
    print "DMG checksum"
    shasum -a 256 "$dmg_path"
else
    print "DMG not present; skipped disk-image verification"
fi
