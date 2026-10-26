#!/usr/bin/env python3
"""
Синхронизирует версию в station/Cargo.toml из station/VERSION.
"""
import re
from pathlib import Path

station_dir = Path(__file__).resolve().parent.parent
version_file = station_dir / "VERSION"
cargo_file = station_dir / "Cargo.toml"

if version_file.exists() and cargo_file.exists():
    ver = version_file.read_text(encoding="utf-8").strip()
    content = cargo_file.read_text(encoding="utf-8")
    new_content = re.sub(r'(?m)^version\s*=\s*["\'][^"\']+["\']', f'version = "{ver}"', content, count=1)
    if new_content != content:
        cargo_file.write_text(new_content, encoding="utf-8")
        print(f"[SyncVersion] station/Cargo.toml -> v{ver}")
