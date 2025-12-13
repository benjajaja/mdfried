#/usr/bin/env bash

set -euo pipefail
if [ $# -lt 1 ]; then
  echo "Usage: $0 <major|minor|patch> [extra arguments to cargo-release]" >&2
  exit 1
fi


level=$1
shift

case "$level" in
  major) new_version="$((major + 1)).0.0" ;;
  minor) new_version="$major.$((minor + 1)).0" ;;
  patch) new_version="$major.$minor.$((patch + 1))" ;;
  *) echo "Invalid level"; exit 1 ;;
esac

release_date=$(date +%Y-%m-%d)

echo -e "\033[33m* Update CHANGELOG \033[0m"
sed -i.bak "s/## \[Unreleased\]/## [Unreleased]\n\n## [$new_version] - $release_date/" CHANGELOG.md
rm CHANGELOG.md.bak
git add CHANGELOG.md
git commit -m "chore: release $new_version"

echo -e "\033[33m* Check flake \033[0m"
nix flake check
echo -e "\033[33m* Release crate \033[0m"
cargo release $level --execute --sign-commit "$@"
echo -e "\033[33m* Push github release \033[0m"
git push origin master --tags

