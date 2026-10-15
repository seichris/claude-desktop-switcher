#!/bin/sh
# Build reviewed local sources without installing a Tauri CLI or accessing
# signing identities. Installation is deliberately separate from packaging.
set -eu
repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo"
cargo build --locked --release -p csw-desktop --features tauri/custom-protocol
cargo build --locked --release -p csw-cli
python3 - <<'PY'
import json, plistlib, shutil
from pathlib import Path

root = Path.cwd()
config = json.loads((root / 'crates/desktop/tauri.conf.json').read_text())
app = root / 'target/release/bundle/macos/Claude-Desktop-Switcher.app'
binary = app / 'Contents/MacOS/csw-desktop'
resources = app / 'Contents/Resources'
binary.parent.mkdir(parents=True, exist_ok=True)
resources.mkdir(parents=True, exist_ok=True)
shutil.copy2(root / 'target/release/csw-desktop', binary)
shutil.copy2(root / 'crates/desktop/icons/icon.icns', resources / 'icon.icns')
info = {
    'CFBundleName': config['productName'],
    'CFBundleDisplayName': 'Claude Desktop Switcher (Local)',
    'CFBundleIdentifier': config['identifier'],
    'CFBundleExecutable': 'csw-desktop',
    'CFBundleIconFile': 'icon.icns',
    'CFBundlePackageType': 'APPL',
    'CFBundleShortVersionString': config['version'],
    'CFBundleVersion': config['version'].split('-')[0],
    'LSMinimumSystemVersion': '11.0',
    'NSHighResolutionCapable': True,
    'LSUIElement': True,
}
(app / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
print(app)
PY
app="$repo/target/release/bundle/macos/Claude-Desktop-Switcher.app"
/usr/bin/codesign --force --sign - "$app"
/usr/bin/codesign --verify --deep --strict "$app"
printf 'Built local app: %s\nCLI: %s\n' "$app" "$repo/target/release/csw"
