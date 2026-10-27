#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
repo_root=${script_dir:h}
package_root="$repo_root/target/package/macos"
app_name="Low Poly Earth"
app_bundle="$package_root/$app_name.app"
dmg_path="$package_root/$app_name-Apple-Silicon.dmg"
plist_template="$repo_root/packaging/macos/Info.plist"
icon_source="$repo_root/packaging/macos/AppIcon.icns"
binary_source="$repo_root/target/release/vearth-app"
app_only=false

case "${1:-}" in
    "")
        ;;
    --app-only)
        app_only=true
        ;;
    *)
        print -u2 "usage: tools/package-macos.sh [--app-only]"
        exit 2
        ;;
esac

if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
    print -u2 "macOS packaging currently requires an Apple-silicon Mac"
    exit 1
fi

version=$(awk '
    $0 == "[workspace.package]" { workspace = 1; next }
    workspace && $0 ~ /^\[/ { exit }
    workspace && $1 == "version" {
        gsub(/"/, "", $3)
        print $3
        exit
    }
' "$repo_root/Cargo.toml")
if [[ -z "$version" ]]; then
    print -u2 "could not read workspace package version"
    exit 1
fi
build_number=${VEARTH_BUILD_NUMBER:-$(git -C "$repo_root" rev-list --count HEAD)}
minimum_macos=$(plutil -extract LSMinimumSystemVersion raw "$plist_template")

# Direct-download distribution: a Developer ID Application identity signs the
# app and DMG; a notarytool keychain profile then notarizes and staples the DMG.
# Both are optional so local builds stay unsigned.
developer_id=${VEARTH_DEVELOPER_ID_IDENTITY:-}
notary_profile=${VEARTH_NOTARY_PROFILE:-}
if [[ -n "$notary_profile" && -z "$developer_id" ]]; then
    print -u2 "VEARTH_NOTARY_PROFILE requires VEARTH_DEVELOPER_ID_IDENTITY"
    exit 1
fi
if [[ -n "$notary_profile" ]] && $app_only; then
    print -u2 "notarization applies to the DMG; omit --app-only"
    exit 1
fi
if [[ -n "$developer_id" ]]; then
    if [[ "$developer_id" != "Developer ID Application:"* ]] ||
        ! security find-identity -v -p codesigning | grep -F "\"$developer_id\"" >/dev/null; then
        print -u2 "no valid Developer ID Application identity named: $developer_id"
        security find-identity -v -p codesigning >&2
        exit 1
    fi
fi
if [[ -n "$notary_profile" ]] &&
    ! xcrun notarytool history --keychain-profile "$notary_profile" >/dev/null; then
    print -u2 "notarytool keychain profile is unavailable: $notary_profile"
    exit 1
fi

python3 "$repo_root/tools/cargo-notices.py" --check
print "Building $app_name $version ($build_number) for Apple silicon"
MACOSX_DEPLOYMENT_TARGET="$minimum_macos" CARGO_TARGET_DIR="$repo_root/target" cargo build \
    --manifest-path "$repo_root/Cargo.toml" \
    --locked --release -p vearth-app --bin vearth-app

# This is a generated-output directory with an exact, repository-owned path.
rm -rf "$package_root"
mkdir -p "$app_bundle/Contents/MacOS" "$app_bundle/Contents/Resources"

ditto "$binary_source" "$app_bundle/Contents/MacOS/$app_name"
chmod 755 "$app_bundle/Contents/MacOS/$app_name"
ditto "$plist_template" "$app_bundle/Contents/Info.plist"
ditto "$icon_source" "$app_bundle/Contents/Resources/AppIcon.icns"
ditto "$repo_root/LICENSE" "$app_bundle/Contents/Resources/LICENSE"
python3 "$repo_root/tools/write-privacy-policy.py" "$app_bundle/Contents/Resources/PrivacyPolicy.html"
ditto "$repo_root/packaging/macos/PrivacyInfo.xcprivacy" "$app_bundle/Contents/Resources/PrivacyInfo.xcprivacy"
ditto "$repo_root/crates/vearth-gen/assets/landmarks/statue_of_liberty-LICENSE.txt" \
    "$app_bundle/Contents/Resources/LICENSE-StatueOfLiberty-MIT.txt"
ditto "$repo_root/crates/vearth-app/assets/fonts/OFL.txt" \
    "$app_bundle/Contents/Resources/OFL-IBMPlex.txt"
ditto "$repo_root/crates/vearth-app/assets/fonts/OFL-NotoSansCJK.txt" \
    "$app_bundle/Contents/Resources/OFL-NotoSansCJK.txt"
python3 "$repo_root/tools/verify-title-fonts.py"
mkdir -p "$app_bundle/Contents/Resources/TitleFontLicenses"
for title_license in "$repo_root"/crates/vearth-app/assets/fonts/titles/*-OFL.txt; do
    ditto "$title_license" "$app_bundle/Contents/Resources/TitleFontLicenses/${title_license:t}"
done
"$repo_root/tools/write-third-party-notices.sh" \
    "$app_bundle/Contents/Resources/THIRD_PARTY_NOTICES.md"

plutil -replace CFBundleShortVersionString -string "$version" \
    "$app_bundle/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$build_number" \
    "$app_bundle/Contents/Info.plist"
plutil -lint "$app_bundle/Contents/Info.plist"

binary_description=$(file "$app_bundle/Contents/MacOS/$app_name")
if [[ "$binary_description" != *"Mach-O 64-bit executable arm64"* ]]; then
    print -u2 "packaged executable is not arm64: $binary_description"
    exit 1
fi
if otool -L "$app_bundle/Contents/MacOS/$app_name" |
    grep -E '/opt/homebrew|/usr/local|@rpath|@loader_path' >/dev/null; then
    print -u2 "packaged executable has a non-system dynamic-library dependency"
    otool -L "$app_bundle/Contents/MacOS/$app_name"
    exit 1
fi

if [[ -n "$developer_id" ]]; then
    # The renderer needs no hardened-runtime exceptions, so no entitlements.
    codesign --force --sign "$developer_id" --options runtime --timestamp "$app_bundle"
    codesign --verify --strict --verbose=2 "$app_bundle"
fi

print "Created $app_bundle"
du -sh "$app_bundle"

if $app_only; then
    exit 0
fi

dmg_stage=$(mktemp -d "$package_root/.dmg-stage.XXXXXX")
trap 'rm -rf "$dmg_stage"' EXIT
ditto "$app_bundle" "$dmg_stage/$app_name.app"
ln -s /Applications "$dmg_stage/Applications"

hdiutil create -ov -fs HFS+ -format UDZO \
    -imagekey zlib-level=9 \
    -volname "$app_name" \
    -srcfolder "$dmg_stage" \
    "$dmg_path"
hdiutil verify "$dmg_path"

if [[ -n "$developer_id" ]]; then
    codesign --force --sign "$developer_id" --timestamp "$dmg_path"
    codesign --verify --strict --verbose=2 "$dmg_path"
fi
if [[ -n "$notary_profile" ]]; then
    # Notarizing the DMG also notarizes the app inside it. `submit --wait`
    # can exit 0 for a rejected submission, so check the reported status.
    notary_result="$dmg_stage/notary.json"
    xcrun notarytool submit "$dmg_path" --keychain-profile "$notary_profile" \
        --wait --output-format json > "$notary_result"
    notary_status=$(plutil -extract status raw "$notary_result")
    if [[ "$notary_status" != Accepted ]]; then
        print -u2 "notarization finished with status: $notary_status"
        xcrun notarytool log "$(plutil -extract id raw "$notary_result")" \
            --keychain-profile "$notary_profile" >&2
        exit 1
    fi
    xcrun stapler staple "$dmg_path"
    xcrun stapler validate "$dmg_path"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg_path"
fi

print "Created $dmg_path"
du -sh "$dmg_path"
shasum -a 256 "$dmg_path"
