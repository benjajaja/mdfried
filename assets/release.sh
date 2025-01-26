#/usr/bin/env bash

set -euo pipefail
if [ $# -lt 1 ]; then
  echo "Usage: $0 <major|minor|patch> [extra arguments to cargo-release]" >&2
  exit 1
fi


level=$1
shift
echo -e "\033[33m* Check flake \033[0m"
nix flake check
echo -e "\033[33m* Release crate \033[0m"
cargo release $level --execute --sign-commit "$@"
echo -e "\033[33m* Push github release \033[0m"
git push origin master --tags

