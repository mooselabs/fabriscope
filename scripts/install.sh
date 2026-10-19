#!/usr/bin/env bash
#
# Installs the latest Fabriscope release.
#
#   curl -fsSL https://raw.githubusercontent.com/mooselabs/fabriscope/main/scripts/install.sh | bash
#
# Settings (environment variables):
#   FABRISCOPE_VERSION       a release tag such as v0.1.0 (default: the latest)
#   FABRISCOPE_INSTALL_DIR   where to put the program (default: ~/.local/bin)
#   FABRISCOPE_RELEASES_URL  where releases are served (default: GitHub)
#
# The archive is checked against the release's SHA256SUMS and nothing is
# installed if it does not match. Everything runs from `main` on the last
# line, so a download cut short runs nothing.

set -euo pipefail

RELEASES_DEFAULT="https://github.com/mooselabs/fabriscope/releases"

say() { printf '==> %s\n' "$*" >&2; }
fail() {
	printf 'fabriscope install: %s\n' "$*" >&2
	exit 1
}

# The release target for this machine.
detect_target() {
	local os arch
	os=$(uname -s)
	arch=$(uname -m)
	case "$os" in
	Linux) os=unknown-linux-musl ;;
	Darwin) os=apple-darwin ;;
	MINGW* | MSYS* | CYGWIN*) fail "Windows is not supported yet. Under WSL, run this inside the Linux shell." ;;
	*) fail "no release for $os. Build from source: cargo install --locked --git https://github.com/mooselabs/fabriscope" ;;
	esac
	case "$arch" in
	x86_64 | amd64) arch=x86_64 ;;
	aarch64 | arm64) arch=aarch64 ;;
	*) fail "no release for $arch. Build from source: cargo install --locked --git https://github.com/mooselabs/fabriscope" ;;
	esac
	# An x86_64 shell under Rosetta on Apple Silicon should get the native build.
	if [ "$os" = apple-darwin ] && [ "$arch" = x86_64 ] &&
		[ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
		arch=aarch64
	fi
	printf '%s-%s\n' "$arch" "$os"
}

# download URL FILE
download() {
	if command -v curl >/dev/null 2>&1; then
		curl --proto '=https,http' -fsSL -o "$2" "$1"
	elif command -v wget >/dev/null 2>&1; then
		wget -q -O "$2" "$1"
	else
		fail "need curl or wget"
	fi
}

# The newest release tag, from where `releases/latest` redirects.
latest_tag() {
	local base=$1 location
	if command -v curl >/dev/null 2>&1; then
		location=$(curl -fsSI "$base/latest" | tr -d '\r' | awk 'tolower($1) == "location:" { print $2 }' | tail -n 1)
	else
		location=$(wget -S --max-redirect=0 -O /dev/null "$base/latest" 2>&1 | tr -d '\r' | awk 'tolower($1) == "location:" { print $2 }' | tail -n 1)
	fi
	case "$location" in
	*/tag/*) printf '%s\n' "${location##*/tag/}" ;;
	*) fail "could not find the latest release at $base/latest (no releases yet?)" ;;
	esac
}

sha256() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | awk '{ print $1 }'
	elif command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | awk '{ print $1 }'
	else
		fail "need sha256sum or shasum to check the download"
	fi
}

main() {
	local base target tag archive dir work expected actual
	base=${FABRISCOPE_RELEASES_URL:-$RELEASES_DEFAULT}
	base=${base%/}
	target=$(detect_target)
	tag=${FABRISCOPE_VERSION:-}
	if [ -z "$tag" ]; then
		tag=$(latest_tag "$base")
	fi
	case "$tag" in v*) ;; *) tag="v$tag" ;; esac
	archive="fabriscope-$target.tar.gz"
	dir=${FABRISCOPE_INSTALL_DIR:-$HOME/.local/bin}

	work=$(mktemp -d)
	# shellcheck disable=SC2064 # expand now: $work is local to main
	trap "rm -rf '$work'" EXIT

	say "Downloading Fabriscope $tag for $target"
	download "$base/download/$tag/$archive" "$work/$archive" ||
		fail "could not download $base/download/$tag/$archive"
	download "$base/download/$tag/SHA256SUMS" "$work/SHA256SUMS" ||
		fail "could not download SHA256SUMS for $tag; not installing an unchecked program"
	expected=$(awk -v name="$archive" '{ file = $2; sub(/^\*/, "", file); if (file == name) { print tolower($1); exit } }' "$work/SHA256SUMS")
	[ -n "$expected" ] || fail "SHA256SUMS for $tag lists no $archive; not installing it"
	actual=$(sha256 "$work/$archive")
	[ "$expected" = "$actual" ] || fail "$archive does not match its SHA256SUMS entry; not installing it"

	tar -xzf "$work/$archive" -C "$work" fabriscope
	mkdir -p "$dir"
	# Replace with one rename, so a running Fabriscope keeps its program.
	cp "$work/fabriscope" "$dir/.fabriscope-install.$$"
	chmod 755 "$dir/.fabriscope-install.$$"
	mv -f "$dir/.fabriscope-install.$$" "$dir/fabriscope"

	say "Installed $("$dir/fabriscope" --version) to $dir/fabriscope"
	case ":$PATH:" in
	*":$dir:"*) ;;
	*) say "$dir is not on your PATH. Add it, for example: export PATH=\"$dir:\$PATH\"" ;;
	esac
	say "It updates itself in the background; set FABRISCOPE_NO_UPDATE=1 to stop that."
}

main "$@"
