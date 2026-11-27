#!/bin/sh
set -eu

# Standalone installer for JustMux (`jm` and `justmux`).
# Downloads the prebuilt platform binary from npm (falling back to npmmirror),
# verifies its SHA-256 digest, installs it atomically, and ensures it is on PATH.

DEFAULT_REGISTRY="https://registry.npmjs.org"
MIRROR_REGISTRY="https://registry.npmmirror.com"

TMP_DIR=""
TMP_BIN=""
TMP_LINK=""

say() {
  printf '%s\n' "$*"
}

warn() {
  printf 'justmux install: warning: %s\n' "$*" >&2
}

die() {
  printf 'justmux install: error: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  if [ -n "$TMP_BIN" ] && [ -e "$TMP_BIN" ]; then
    rm -f "$TMP_BIN" || true
  fi
  if [ -n "$TMP_LINK" ] && [ -e "$TMP_LINK" ]; then
    rm -f "$TMP_LINK" || true
  fi
  if [ -n "$TMP_DIR" ] && [ -d "$TMP_DIR" ]; then
    rm -rf "$TMP_DIR" || true
  fi
}

trap cleanup EXIT INT TERM HUP

detect_platform() {
  os_raw=$(uname -s)
  arch_raw=$(uname -m)

  case "$os_raw" in
    Darwin) platform="darwin" ;;
    Linux) platform="linux" ;;
    *) die "unsupported operating system '$os_raw'. JustMux supports macOS ARM64 and Linux x64." ;;
  esac

  case "$arch_raw" in
    x86_64|amd64) arch="x64" ;;
    arm64|aarch64) arch="arm64" ;;
    *) die "unsupported CPU architecture '$arch_raw'. JustMux supports macOS ARM64 and Linux x64." ;;
  esac

  target="${platform}-${arch}"
  case "$target" in
    darwin-arm64|linux-x64) ;;
    *) die "unsupported platform '$target'. Prebuilt binaries support macOS ARM64 and Linux x64 (or build from source with Cargo)." ;;
  esac

  PKG_NAME="justmux-${target}"
}

require_http_client() {
  if command -v curl >/dev/null 2>&1; then
    HTTP_CLIENT="curl"
  elif command -v wget >/dev/null 2>&1; then
    HTTP_CLIENT="wget"
  else
    die "curl or wget is required to download JustMux."
  fi
}

require_sha256_tool() {
  if command -v sha256sum >/dev/null 2>&1; then
    SHA_TOOL="sha256sum"
  elif command -v shasum >/dev/null 2>&1; then
    SHA_TOOL="shasum"
  elif command -v openssl >/dev/null 2>&1; then
    SHA_TOOL="openssl"
  else
    die "a SHA-256 verification tool (sha256sum, shasum, or openssl) is required."
  fi
}

sha256_file() {
  file="$1"
  case "$SHA_TOOL" in
    sha256sum)
      sha256sum "$file" | awk '{print $1}'
      ;;
    shasum)
      shasum -a 256 "$file" | awk '{print $1}'
      ;;
    openssl)
      openssl dgst -sha256 "$file" | awk '{print $NF}'
      ;;
  esac
}

validate_registry() {
  reg="${1%/}"
  case "$reg" in
    https://?*) ;;
    http://127.0.0.1|http://127.0.0.1:*|http://localhost|http://localhost:*) ;;
    *) die "JUSTMUX_REGISTRY must use https:// (got '$1')." ;;
  esac
  printf '%s' "$reg"
}

registry_origin() {
  case "$1" in
    https://*)
      rest="${1#https://}"
      printf 'https://%s' "${rest%%/*}"
      ;;
    http://*)
      rest="${1#http://}"
      printf 'http://%s' "${rest%%/*}"
      ;;
  esac
}

validate_semver() {
  v="$1"
  printf '%s' "$v" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$' || return 1
  case "$v" in
    *-*)
      pre="${v#*-}"
      old_ifs="$IFS"
      IFS='.'
      for part in $pre; do
        case "$part" in
          0[0-9]*)
            case "$part" in
              *[!0-9]*) ;;
              *) IFS="$old_ifs"; return 1 ;;
            esac
            ;;
        esac
      done
      IFS="$old_ifs"
      ;;
  esac
  return 0
}

fetch_url() {
  url="$1"
  dest="$2"
  timeout="${3:-30}"

  case "$url" in
    https://*) ;;
    http://127.0.0.1[:/]*|http://localhost[:/]*) ;;
    *) die "refusing insecure URL: $url" ;;
  esac

  if [ "$HTTP_CLIENT" = "curl" ]; then
    case "$url" in
      https://*)
        curl -fsSL --proto '=https' --proto-redir '=https' --connect-timeout 15 --max-time "$timeout" -o "$dest" "$url"
        ;;
      *)
        curl -fsSL --connect-timeout 15 --max-time "$timeout" -o "$dest" "$url"
        ;;
    esac
  else
    wget -q -T "$timeout" -O "$dest" "$url"
  fi
}

json_get() {
  file="$1"
  key1="$2"
  key2="${3:-}"

  if command -v python3 >/dev/null 2>&1; then
    python3 - "$file" "$key1" "$key2" <<'PY'
import json, sys
with open(sys.argv[1], 'r', encoding='utf-8') as f:
    data = json.load(f)
val = data.get(sys.argv[2]) if isinstance(data, dict) else None
if sys.argv[3]:
    val = val.get(sys.argv[3]) if isinstance(val, dict) else None
if isinstance(val, str):
    sys.stdout.write(val)
else:
    sys.exit(1)
PY
    return $?
  fi

  if command -v node >/dev/null 2>&1; then
    node -e '
      const fs = require("fs");
      const [file, k1, k2] = process.argv.slice(1);
      const data = JSON.parse(fs.readFileSync(file, "utf8"));
      const val = k2 ? data?.[k1]?.[k2] : data?.[k1];
      if (typeof val !== "string") process.exit(1);
      process.stdout.write(val);
    ' "$file" "$key1" "$key2"
    return $?
  fi

  awk -v want1="$key1" -v want2="$key2" '
    function hexval(c,    idx) {
      idx = index("0123456789abcdefABCDEF", c)
      if (idx == 0) return -1
      idx--
      if (idx >= 16) idx -= 6
      return idx
    }
    BEGIN { src = ""; found = 0 }
    { src = src $0 "\n" }
    END {
      n = length(src)
      depth = 0
      i = 1
      while (i <= n) {
        c = substr(src, i, 1)
        if (c == " " || c == "\t" || c == "\n" || c == "\r") {
          i++
          continue
        }
        if (c == "{") {
          depth++
          stack[depth] = "obj"
          expect_key[depth] = 1
          expect_val[depth] = 0
          keys[depth] = ""
          i++
          continue
        }
        if (c == "[") {
          depth++
          stack[depth] = "arr"
          expect_key[depth] = 0
          expect_val[depth] = 0
          keys[depth] = ""
          i++
          continue
        }
        if (c == "}" || c == "]") {
          if (depth > 0) {
            depth--
            expect_val[depth] = 0
          }
          i++
          continue
        }
        if (c == ":") {
          if (depth > 0 && stack[depth] == "obj") {
            expect_key[depth] = 0
            expect_val[depth] = 1
          }
          i++
          continue
        }
        if (c == ",") {
          if (depth > 0 && stack[depth] == "obj") {
            expect_key[depth] = 1
            expect_val[depth] = 0
            keys[depth] = ""
          }
          i++
          continue
        }
        if (c == "\"") {
          i++
          val = ""
          while (i <= n) {
            ch = substr(src, i, 1)
            if (ch == "\"") {
              i++
              break
            }
            if (ch == "\\") {
              i++
              if (i > n) break
              esc = substr(src, i, 1)
              if (esc == "\"" || esc == "\\" || esc == "/") val = val esc
              else if (esc == "n") val = val "\n"
              else if (esc == "r") val = val "\r"
              else if (esc == "t") val = val "\t"
              else if (esc == "b") val = val "\b"
              else if (esc == "f") val = val "\f"
              else if (esc == "u") {
                if (i + 4 <= n) {
                  h1 = hexval(substr(src, i + 1, 1))
                  h2 = hexval(substr(src, i + 2, 1))
                  h3 = hexval(substr(src, i + 3, 1))
                  h4 = hexval(substr(src, i + 4, 1))
                  if (h1 >= 0 && h2 >= 0 && h3 >= 0 && h4 >= 0) {
                    code = ((h1 * 16 + h2) * 16 + h3) * 16 + h4
                    if (code >= 32 && code < 127) val = val sprintf("%c", code)
                    else val = val "\\u" substr(src, i + 1, 4)
                    i += 4
                  }
                }
              } else {
                val = val esc
              }
            } else {
              val = val ch
            }
            i++
          }
          if (depth > 0 && stack[depth] == "obj") {
            if (expect_key[depth] == 1) {
              keys[depth] = val
              expect_key[depth] = 0
            } else if (expect_val[depth] == 1) {
              if ((want2 == "" && depth == 1 && keys[1] == want1) \
                || (want2 != "" && depth == 2 && keys[1] == want1 && keys[2] == want2)) {
                printf "%s", val
                found = 1
                exit 0
              }
              expect_val[depth] = 0
            }
          }
          continue
        }
        i++
      }
      if (!found) exit 1
    }
  ' "$file"
}

normalize_dir() {
  d="$1"
  while :; do
    case "$d" in
      /) printf '/'; return 0 ;;
      */) d="${d%/}" ;;
      *) printf '%s' "$d"; return 0 ;;
    esac
  done
}

path_contains_dir() {
  target_dir=$(normalize_dir "$1")
  old_ifs="$IFS"
  IFS=':'
  for entry in ${PATH:-}; do
    [ -n "$entry" ] || continue
    norm_entry=$(normalize_dir "$entry")
    if [ "$norm_entry" = "$target_dir" ]; then
      IFS="$old_ifs"
      return 0
    fi
  done
  IFS="$old_ifs"
  return 1
}

try_path_dir() {
  dir="$1"
  home_dir="$2"
  path_contains_dir "$dir" || return 1
  if [ -d "$dir" ] && [ -w "$dir" ] && [ -x "$dir" ]; then
    INSTALL_DIR="$dir"
    return 0
  fi
  if [ ! -e "$dir" ] && [ -n "$home_dir" ] && { [ "$dir" = "${home_dir}/.local/bin" ] || [ "$dir" = "${home_dir}/bin" ]; }; then
    if mkdir -p "$dir" 2>/dev/null && [ -w "$dir" ] && [ -x "$dir" ]; then
      INSTALL_DIR="$dir"
      return 0
    fi
  fi
  return 1
}

choose_install_dir() {
  if [ -n "${JUSTMUX_INSTALL_DIR:-}" ]; then
    dir=$(normalize_dir "$JUSTMUX_INSTALL_DIR")
    mkdir -p "$dir" || die "cannot create install directory '$dir'."
    [ -d "$dir" ] && [ -w "$dir" ] && [ -x "$dir" ] || die "install directory '$dir' is not writable."
    INSTALL_DIR="$dir"
    return 0
  fi

  home_dir="${HOME:-}"
  skip_system="${JUSTMUX_SKIP_SYSTEM_DIRS:-0}"

  if [ "$skip_system" != "1" ]; then
    if try_path_dir "/usr/local/bin" "$home_dir"; then return 0; fi
    if try_path_dir "/opt/homebrew/bin" "$home_dir"; then return 0; fi
  fi

  if [ -n "$home_dir" ]; then
    if try_path_dir "${home_dir}/.local/bin" "$home_dir"; then return 0; fi
    if try_path_dir "${home_dir}/bin" "$home_dir"; then return 0; fi
  fi

  if [ "$skip_system" != "1" ] && [ -d "/usr/local/bin" ] && [ -w "/usr/local/bin" ] && [ -x "/usr/local/bin" ]; then
    INSTALL_DIR="/usr/local/bin"
    return 0
  fi

  [ -n "$home_dir" ] || die "HOME is not set and /usr/local/bin is not writable. Set JUSTMUX_INSTALL_DIR."
  INSTALL_DIR="${home_dir}/.local/bin"
  mkdir -p "$INSTALL_DIR" || die "cannot create install directory '$INSTALL_DIR'."
  [ -w "$INSTALL_DIR" ] && [ -x "$INSTALL_DIR" ] || die "install directory '$INSTALL_DIR' is not writable."
}

append_path_to_rc() {
  dir="$1"
  [ "${JUSTMUX_NO_MODIFY_PATH:-0}" = "1" ] && return 1
  home_dir="${HOME:-}"
  [ -n "$home_dir" ] || return 1

  shell_name=$(basename "${SHELL:-sh}")
  case "$shell_name" in
    zsh)
      rc_dir="${ZDOTDIR:-$home_dir}"
      rc_file="${rc_dir}/.zshrc"
      ;;
    bash)
      rc_file="${home_dir}/.bashrc"
      ;;
    *)
      rc_file="${home_dir}/.profile"
      ;;
  esac

  if [ -f "$rc_file" ] && grep -Fq "$dir" "$rc_file"; then
    RC_MODIFIED_FILE="$rc_file"
    RC_STATUS="already-configured"
    return 0
  fi

  rc_parent=$(dirname "$rc_file")
  mkdir -p "$rc_parent" 2>/dev/null || return 1
  if {
    printf '\n# Added by JustMux installer\n'
    printf 'export PATH="%s:$PATH"\n' "$dir"
  } >> "$rc_file" 2>/dev/null; then
    RC_MODIFIED_FILE="$rc_file"
    RC_STATUS="updated"
    return 0
  fi
  return 1
}

main() {
  detect_platform
  require_http_client
  require_sha256_tool
  command -v tar >/dev/null 2>&1 || die "tar is required to unpack JustMux."

  TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/justmux-install.XXXXXX")
  TAGS_FILE="${TMP_DIR}/dist-tags.json"
  META_FILE="${TMP_DIR}/metadata.json"
  TARBALL_FILE="${TMP_DIR}/package.tgz"
  EXTRACT_DIR="${TMP_DIR}/extract"

  if [ -n "${JUSTMUX_REGISTRY:-}" ]; then
    REGISTRIES=$(validate_registry "$JUSTMUX_REGISTRY")
  else
    REGISTRIES="$DEFAULT_REGISTRY $MIRROR_REGISTRY"
  fi

  REQUESTED_VERSION=""
  if [ -n "${JUSTMUX_VERSION:-}" ]; then
    REQUESTED_VERSION="${JUSTMUX_VERSION#v}"
    validate_semver "$REQUESTED_VERSION" || die "invalid JUSTMUX_VERSION '$JUSTMUX_VERSION'."
  fi

  SELECTED_REGISTRY=""
  VERSION=""
  for reg in $REGISTRIES; do
    if [ -n "$REQUESTED_VERSION" ]; then
      if fetch_url "${reg}/${PKG_NAME}/${REQUESTED_VERSION}" "$META_FILE" 20 2>/dev/null; then
        VERSION="$REQUESTED_VERSION"
        SELECTED_REGISTRY="$reg"
        break
      fi
    else
      if fetch_url "${reg}/-/package/justmux/dist-tags" "$TAGS_FILE" 20 2>/dev/null; then
        candidate_version=$(json_get "$TAGS_FILE" "latest" || true)
        if [ -n "$candidate_version" ] && validate_semver "$candidate_version"; then
          if fetch_url "${reg}/${PKG_NAME}/${candidate_version}" "$META_FILE" 20 2>/dev/null; then
            VERSION="$candidate_version"
            SELECTED_REGISTRY="$reg"
            break
          fi
        fi
      fi
    fi
  done

  [ -n "$SELECTED_REGISTRY" ] && [ -n "$VERSION" ] || die "could not download JustMux metadata from npm."

  TARBALL_URL=$(json_get "$META_FILE" "dist" "tarball" || true)
  META_SHA=$(json_get "$META_FILE" "justmux" "sha256" || true)

  [ -n "$TARBALL_URL" ] || die "release metadata is missing dist.tarball."
  [ -n "$META_SHA" ] || die "release metadata is missing justmux.sha256."
  printf '%s' "$META_SHA" | grep -Eq '^[0-9a-f]{64}$' || die "invalid SHA-256 digest in registry metadata."

  ORIGIN=$(registry_origin "$SELECTED_REGISTRY")
  case "$TARBALL_URL" in
    "${ORIGIN}"/*) ;;
    *) die "tarball URL '$TARBALL_URL' does not match selected registry origin '$ORIGIN'." ;;
  esac
  printf '%s' "$TARBALL_URL" | grep -Eq '^https?://[0-9A-Za-z._~:/?#@!$&'\''()*+,;=%-]+$' \
    || die "tarball URL contains unexpected characters."

  say "Downloading JustMux ${VERSION} (${target})..."
  fetch_url "$TARBALL_URL" "$TARBALL_FILE" 120 || die "failed to download package tarball from '$TARBALL_URL'."

  mkdir -p "$EXTRACT_DIR"
  tar -xzf "$TARBALL_FILE" -C "$EXTRACT_DIR" package/bin/jm package/package.json \
    || die "failed to extract JustMux binary from tarball."

  EXTRACTED_BIN="${EXTRACT_DIR}/package/bin/jm"
  EXTRACTED_PKG="${EXTRACT_DIR}/package/package.json"
  [ -f "$EXTRACTED_BIN" ] || die "package tarball is missing package/bin/jm."
  [ -f "$EXTRACTED_PKG" ] || die "package tarball is missing package/package.json."

  PKG_SHA=$(json_get "$EXTRACTED_PKG" "justmux" "sha256" || true)
  [ "$PKG_SHA" = "$META_SHA" ] || die "package.json SHA-256 mismatch against registry metadata."

  ACTUAL_SHA=$(sha256_file "$EXTRACTED_BIN")
  [ "$ACTUAL_SHA" = "$META_SHA" ] || die "SHA-256 verification failed for downloaded binary (expected ${META_SHA}, got ${ACTUAL_SHA})."

  choose_install_dir

  TMP_BIN="${INSTALL_DIR}/.jm.tmp.$$"
  TMP_LINK="${INSTALL_DIR}/.justmux.tmp.$$"
  cp "$EXTRACTED_BIN" "$TMP_BIN"
  chmod 0755 "$TMP_BIN"
  mv -f "$TMP_BIN" "${INSTALL_DIR}/jm"
  TMP_BIN=""

  ln -s jm "$TMP_LINK"
  mv -f "$TMP_LINK" "${INSTALL_DIR}/justmux"
  TMP_LINK=""

  VERSION_OUT=$("${INSTALL_DIR}/jm" --version) || die "installed binary failed verification check."
  say "Installed ${VERSION_OUT} to ${INSTALL_DIR} (commands: jm, justmux)."

  if path_contains_dir "$INSTALL_DIR"; then
    say "Run 'jm' (or 'justmux') to get started."
  else
    RC_MODIFIED_FILE=""
    RC_STATUS=""
    if append_path_to_rc "$INSTALL_DIR"; then
      if [ "$RC_STATUS" = "updated" ]; then
        say "Added ${INSTALL_DIR} to PATH in ${RC_MODIFIED_FILE}."
        say "Run 'export PATH=\"${INSTALL_DIR}:\$PATH\"' (or restart your shell), then run 'jm'."
      else
        say "${INSTALL_DIR} is already referenced in ${RC_MODIFIED_FILE}."
        say "Run 'export PATH=\"${INSTALL_DIR}:\$PATH\"' in your current shell, then run 'jm'."
      fi
    else
      say "Add ${INSTALL_DIR} to your PATH to run 'jm' directly:"
      say "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    fi
  fi

  if ! command -v tmux >/dev/null 2>&1; then
    say "Note: tmux was not found on PATH. Install tmux via your package manager before running jm."
  fi
}

main "$@"
