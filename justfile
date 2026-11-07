set shell := ["bash", "-uc"]

bin := "target/release/vase"
host_crates := if os() == "macos" { "-p vase-core -p vase-macos" } else if os() == "windows" { "-p vase-core -p vase-windows" } else { "-p vase-core" }
app_crate := if os() == "macos" { "vase-macos" } else { "vase-windows" }
windows_target := "x86_64-pc-windows-gnu"
app := "dist/vase.app"
version := `cd vase-macos && cargo read-manifest | jq -r .version`

default:
    @just --list

build:
    cargo build --release -p {{ app_crate }} --bin vase

test:
    cargo test {{ host_crates }}

check:
    cargo clippy {{ host_crates }} --all-targets -- -D warnings
    cargo fmt --check

# Type-check vase-windows from macOS or Linux. Needs mingw-w64 and `rustup target add x86_64-pc-windows-gnu`.
check-windows:
    cargo clippy -p vase-windows --target {{ windows_target }} --all-targets -- -D warnings

# Assemble dist/vase.app around an already-built binary at `binpath`.
_bundle binpath ver:
    rm -rf "{{ app }}"
    mkdir -p "{{ app }}/Contents/MacOS" "{{ app }}/Contents/Resources"
    cp "{{ binpath }}" "{{ app }}/Contents/MacOS/vase"
    chmod +x "{{ app }}/Contents/MacOS/vase"
    cp assets/vase.icns "{{ app }}/Contents/Resources/vase.icns"
    sed "s/__VERSION__/{{ ver }}/g" assets/Info.plist > "{{ app }}/Contents/Info.plist"
    # Ad-hoc sign last (seals the bundle) so Accessibility/Input-Monitoring grants attach: TCC keys grants to a code signature.
    codesign --force --deep --sign - "{{ app }}"

# Build for the host arch and bundle it. Try it locally: `just app && open dist/vase.app`
app ver=version: build
    @just _bundle {{ bin }} {{ ver }}
    @echo "built {{ app }} (v{{ ver }})"

# Write the full CHANGELOG.md, treating HEAD as the given version.
changelog ver=version:
    git-cliff --tag v{{ ver }} --tag-pattern 'v.*' --config cliff.toml --output CHANGELOG.md
