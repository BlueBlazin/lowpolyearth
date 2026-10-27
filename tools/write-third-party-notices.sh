#!/bin/zsh
set -euo pipefail

if [[ $# -ne 1 || -z "$1" ]]; then
    print -u2 "usage: tools/write-third-party-notices.sh OUTPUT"
    exit 2
fi

script_dir=${0:A:h}
repo_root=${script_dir:h}
output=$1
sources=(
    packaging/licenses/CARGO_DEPENDENCY_NOTICES.md
    crates/vearth-app/assets/fonts/README.md
    crates/vearth-app/assets/ui/SOURCES.md
    crates/vearth-app/assets/ui/LICENSE
    crates/vearth-app/assets/mercator-map-SOURCE.md
    crates/vearth-app/assets/sky/SOURCES.md
    crates/vearth-data/assets/admin-regions-SOURCES.md
    crates/vearth-data/assets/climate-SOURCES.md
    crates/vearth-data/assets/place-search-SOURCES.md
    crates/vearth-data/assets/settlement-SOURCES.md
    crates/vearth-data/assets/surface-SOURCES.md
    crates/vearth-data/assets/terrain-SOURCES.md
    crates/vearth-data/assets/timezones-SOURCES.md
    crates/vearth-gen/assets/landmarks/SOURCES.md
    crates/vearth-gen/assets/architecture/places-SOURCES.md
    crates/vearth-gen/assets/water/SOURCES.md
    crates/vearth-gen/assets/landmarks/statue_of_liberty-LICENSE.txt
)

python3 "$repo_root/tools/cargo-notices.py" --check

mkdir -p "${output:h}"
{
    print '# Third-party notices'
    print
    print 'Generated from the dependency notices and asset manifests in the Low Poly Earth source tree.'
    for source in $sources; do
        print
        print -- '---'
        print
        print "Source manifest: \`$source\`"
        print
        command cat "$repo_root/$source"
    done
} > "$output"
