"""Generate real CLI units using only the committed command-test transports.

Usage: python3 generate.py /absolute/leptoskit /new/output/directory
Run from the repository root. The output directory must not exist.
"""

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

binary = Path(sys.argv[1]).resolve()
root = Path(sys.argv[2]).resolve()
root.mkdir()
source = Path("crates/leptoskit_cli/tests/infra.rs").read_text()
for directory in ("src", "migrations", "bin"):
    (root / directory).mkdir()
(root / "Cargo.toml").write_text("[package]\nname='notes'\nversion='0.1.0'\n")
(root / "src/main.rs").write_text("fn main() { /* readyz */ }")
(root / "migrations/registry.txt").write_text("")
(root / "leptoskit.toml").write_text(
    re.search(r'const CONFIG: &str = r#"(.*?)"#;', source, re.S)[1]
)
for name, script in re.findall(
    r'tool\(\s*root.path\(\),\s*"([^" ]+)",\s*r#"(.*?)"#,', source, re.S
)[:4]:
    path = root / "bin" / name
    path.write_text(script)
    path.chmod(0o755)
env = {"PATH": str(root / "bin") + os.pathsep + os.environ["PATH"],
       "HOME": str(root), "MOCK_REMOTE_ROOT": str(root / "remote"),
       "MOCK_SCHEMA_LABEL": hashlib.sha256(b"[]").hexdigest()}

for key in ("RUSTUP_HOME", "RUSTUP_TOOLCHAIN", "CARGO_HOME"):
    if key in os.environ:
        env[key] = os.environ[key]

for key in ("LEPTOSKIT_BACKUP_PASSWORD", "LEPTOSKIT_BACKUP_ACCESS_KEY_ID",
            "LEPTOSKIT_BACKUP_SECRET_ACCESS_KEY", "POSTGRES_PASSWORD",
            "DATABASE_PASSWORD", "KAMAL_REGISTRY_PASSWORD",
            "NATS_TOKEN", "VALKEY_PASSWORD"):
    env[key] = "fixture-only-not-a-credential"

for args in (("infra", "generate"), ("infra", "backups", "install")):
    subprocess.run([str(binary), *args], cwd=root, env=env, check=True)
print(json.dumps({"binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                  "units": str(root / "remote/etc/systemd/system")}))
