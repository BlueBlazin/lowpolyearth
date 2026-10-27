#!/bin/zsh
set -euo pipefail

script_dir=${0:A:h}
repo_root=${script_dir:h}
package_root="$repo_root/target/package/mac-app-store"
app_bundle="$package_root/Low Poly Earth.app"
package_path="$package_root/LowPolyEarth.pkg"
template="$repo_root/packaging/macos/AppStore.entitlements"
prepare=false

case "${1:-}" in
    "") ;;
    --prepare) prepare=true ;;
    *)
        print -u2 "usage: tools/package-macos-app-store.sh [--prepare]"
        exit 2
        ;;
esac
if [[ $# -gt 1 ]]; then
    print -u2 "usage: tools/package-macos-app-store.sh [--prepare]"
    exit 2
fi
if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
    print -u2 "App Store packaging requires an Apple-silicon Mac"
    exit 1
fi

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/vearth-app-store.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT
entitlements="$work_dir/AppStore.entitlements"
bundle_id=$(plutil -extract CFBundleIdentifier raw "$repo_root/packaging/macos/Info.plist")

if $prepare; then
    # Preparation seals a sandboxed local bundle without needing distribution
    # credentials. It never produces an uploadable installer package.
    app_identity=${VEARTH_APP_SIGN_IDENTITY:--}
    ditto "$template" "$entitlements"
else
    : "${VEARTH_APP_STORE_PROFILE:?Set VEARTH_APP_STORE_PROFILE to a Mac App Store Connect profile}"
    : "${VEARTH_APP_SIGN_IDENTITY:?Set VEARTH_APP_SIGN_IDENTITY to an App Store application signing identity}"
    : "${VEARTH_INSTALLER_SIGN_IDENTITY:?Set VEARTH_INSTALLER_SIGN_IDENTITY to a Mac Installer Distribution identity}"
    if [[ ! -f "$VEARTH_APP_STORE_PROFILE" ]]; then
        print -u2 "provisioning profile not found: $VEARTH_APP_STORE_PROFILE"
        exit 1
    fi
    # Validate credentials before replacing any generated package. The copy
    # embedded below is the same profile that passed preflight.
    ditto "$VEARTH_APP_STORE_PROFILE" "$work_dir/embedded.provisionprofile"
    python3 "$script_dir/app-store-entitlements.py" \
        --profile "$work_dir/embedded.provisionprofile" \
        --template "$template" --bundle-id "$bundle_id" \
        --app-identity "$VEARTH_APP_SIGN_IDENTITY" \
        --installer-identity "$VEARTH_INSTALLER_SIGN_IDENTITY" \
        --output "$entitlements"
    app_identity=$VEARTH_APP_SIGN_IDENTITY
fi

"$script_dir/package-macos.sh" --app-only
"$script_dir/verify-macos-package.sh"
# Only this repository-owned generated-output directory is replaced.
rm -rf "$package_root"
mkdir -p "$package_root"
ditto "$repo_root/target/package/macos/Low Poly Earth.app" "$app_bundle"
if ! $prepare; then
    ditto "$work_dir/embedded.provisionprofile" "$app_bundle/Contents/embedded.provisionprofile"
fi

# No JIT, unsigned-memory, or library-validation exceptions are needed by the
# native Rust/Metal renderer. Signing does not execute the app.
codesign --force --sign "$app_identity" --options runtime \
    --entitlements "$entitlements" "$app_bundle"
codesign --verify --strict --verbose=2 "$app_bundle"
codesign --display --entitlements - --xml "$app_bundle" > "$work_dir/signed-entitlements.plist"
python3 - "$entitlements" "$work_dir/signed-entitlements.plist" <<'PY'
import plistlib
import sys
from pathlib import Path

expected, actual = (plistlib.loads(Path(path).read_bytes()) for path in sys.argv[1:])
if actual != expected:
    raise SystemExit("Signed entitlements do not match the requested App Store entitlements")
PY

if $prepare; then
    print "Prepared sandboxed app: $app_bundle"
    print "Local preparation only; no App Store installer was created or uploaded."
    exit 0
fi

productbuild --sign "$VEARTH_INSTALLER_SIGN_IDENTITY" \
    --component "$app_bundle" /Applications "$package_path"
pkgutil --check-signature "$package_path"
print "Created locally: $package_path"
print "Nothing has been uploaded or submitted."
