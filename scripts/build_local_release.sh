#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# build_local_release.sh
# -----------------------------------------------------------------------------
# Builds a local release version of the Codex CLI
#
# Usage:
#   ./build_local_release.sh [options]
#
# Options:
#   --target <TARGET>  : Build for specific target (default: native)
#   --output <DIR>     : Output directory (default: ./release)
#   --version <VER>    : Set version (default: 0.0.0-local)
#   -h, --help         : Print this help message
#
# Examples:
#   ./build_local_release.sh
#   ./build_local_release.sh --target x86_64-apple-darwin
#   ./build_local_release.sh --output ~/codex-release --version 0.1.0-test
# -----------------------------------------------------------------------------

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Default values
TARGET=""
VERSION="0.0.0-local"
CLEAN_BUILD=false
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
OUTPUT_DIR="$PROJECT_ROOT/dist"
RUST_ROOT="$PROJECT_ROOT/codex-rs"
CLI_ROOT="$PROJECT_ROOT/codex-cli"

# Function to print colored output
print_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
print_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Function to print usage
usage() {
  cat <<EOF
Usage: $(basename "$0") [options]

Options:
  --target <TARGET>  Build for specific target (default: native)
  --output <DIR>     Output directory (default: PROJECT_ROOT/dist)
  --version <VER>    Set version (default: 0.0.0-local)
  --clean            Force clean rebuild (removes target directory)
  -h, --help         Print this help message

Supported targets:
  - x86_64-apple-darwin     (Intel macOS)
  - aarch64-apple-darwin    (Apple Silicon macOS)
  - x86_64-unknown-linux-gnu (Linux x64)
  - x86_64-pc-windows-msvc  (Windows x64)

Examples:
  $(basename "$0")
  $(basename "$0") --target x86_64-apple-darwin
  $(basename "$0") --output ~/codex-release --version 0.1.0-test
  $(basename "$0") --clean                    # Force clean rebuild
EOF
  exit "${1:-0}"
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      shift
      TARGET="$1"
      ;;
    --output)
      shift
      OUTPUT_DIR="$1"
      ;;
    --version)
      shift
      VERSION="$1"
      ;;
    --clean)
      CLEAN_BUILD=true
      ;;
    -h|--help)
      usage 0
      ;;
    *)
      print_error "Unknown option: $1"
      usage 1
      ;;
  esac
  shift
done

# Create output directory
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"

print_info "Building Codex CLI local release"
print_info "Version: $VERSION"
print_info "Output: $OUTPUT_DIR"

# Change to rust project directory
cd "$RUST_ROOT"

# Clean build if requested
if [[ "$CLEAN_BUILD" == "true" ]]; then
  print_info "Cleaning previous build artifacts..."
  # cargo clean
fi

# Install required toolchain if needed
print_info "Checking Rust toolchain..."
rustup show active-toolchain

# Force rebuild by touching source files


# Build the release
if [[ -n "$TARGET" ]]; then
  print_info "Building for target: $TARGET"
  
  # Add target if not already installed
  rustup target add "$TARGET" 2>/dev/null || true
  
  # Build with specific target (only the main codex binary)
  cargo build --release --target "$TARGET" --bin codex
  
  # Determine binary paths based on target
  BINARY_DIR="target/$TARGET/release"
else
  print_info "Building for native target..."
  
  # Build for native target (only the main codex binary)
  cargo build --release --bin codex
  
  BINARY_DIR="target/release"
fi

# Copy binary to output directory
print_info "Copying binary to output directory..."

# Determine the platform suffix based on target
if [[ -n "$TARGET" ]]; then
  PLATFORM_SUFFIX="$TARGET"
else
  # Auto-detect platform
  case "$OSTYPE" in
    darwin*)
      if [[ "$(uname -m)" == "arm64" ]]; then
        PLATFORM_SUFFIX="aarch64-apple-darwin"
      else
        PLATFORM_SUFFIX="x86_64-apple-darwin"
      fi
      ;;
    linux*)
      PLATFORM_SUFFIX="x86_64-unknown-linux-gnu"
      ;;
    msys*|cygwin*|mingw*)
      PLATFORM_SUFFIX="x86_64-pc-windows-msvc"
      ;;
    *)
      PLATFORM_SUFFIX="unknown"
      ;;
  esac
fi

# Create bin directory and copy platform-specific binary
mkdir -p "$OUTPUT_DIR/bin"

if [[ "$OSTYPE" == "mswin"* ]] || [[ "$OSTYPE" == "cygwin"* ]] || [[ -n "${TARGET:-}" && "$TARGET" == *"windows"* ]]; then
  # Windows binary has .exe extension
  cp "$BINARY_DIR/codex.exe" "$OUTPUT_DIR/bin/codex-${PLATFORM_SUFFIX}.exe" 2>/dev/null || print_error "codex.exe not found"
else
  # Unix-like systems
  cp "$BINARY_DIR/codex" "$OUTPUT_DIR/bin/codex-${PLATFORM_SUFFIX}" 2>/dev/null || print_error "codex not found"
  
  # Make binary executable
  chmod +x "$OUTPUT_DIR/bin/codex-${PLATFORM_SUFFIX}" 2>/dev/null || true
fi

# Copy JavaScript CLI files
print_info "Setting up JavaScript CLI wrapper..."
cp "$CLI_ROOT/bin/codex.js" "$OUTPUT_DIR/bin/"

# Create package.json for the dist
cat > "$OUTPUT_DIR/package.json" << EOF
{
  "name": "@openai/codex",
  "version": "$VERSION",
  "description": "Codex CLI - Local Build",
  "type": "module",
  "bin": {
    "codex": "bin/codex.js"
  },
  "files": [
    "bin",
    "native",
    "README.md"
  ],
  "engines": {
    "node": ">=18"
  }
}
EOF

# Create version file
echo "$VERSION" > "$OUTPUT_DIR/VERSION"

# Copy README if exists
if [[ -f "$PROJECT_ROOT/README.md" ]]; then
  cp "$PROJECT_ROOT/README.md" "$OUTPUT_DIR/"
fi

# Print summary
print_info "Build complete!"
print_info "Distribution located at: $OUTPUT_DIR"
echo
echo "Directory structure:"
echo "  $OUTPUT_DIR/"
echo "  ├── bin/"
echo "  │   ├── codex.js"
echo "  │   └── codex-${PLATFORM_SUFFIX}"
echo "  ├── package.json"
echo "  ├── README.md"
echo "  └── VERSION"
echo
echo "To use the local build:"
echo "  1. Via npm link:"
echo "     cd $OUTPUT_DIR && npm link"
echo "     codex --version"
echo
echo "  2. Direct execution:"
echo "     node $OUTPUT_DIR/bin/codex.js --version"
echo
echo "  3. Install globally from dist:"
echo "     npm install -g $OUTPUT_DIR"

# Test the binary
print_info "Testing build..."
if command -v node >/dev/null 2>&1; then
  node "$OUTPUT_DIR/bin/codex.js" --version || print_error "Failed to run codex"
else
  print_warn "Node.js not found. Cannot test the CLI wrapper."
fi

print_info "Local release build completed successfully!"