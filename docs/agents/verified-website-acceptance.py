"""Real website acceptance; run after committing the fixture.

Arguments: CLI, digest-pinned builder, digest-pinned runtime, optional fixture tree-ish. CONTAINER_CONNECTION
selects the task-owned Podman VM. Builder needs Rust+wasm target, leptoskit and
wasm-opt installed. No registry publication or deployment occurs in this script.
Local signed fixture commit + invocation-only Git transport mapping tests the
full reproduction mechanics, not independently authenticated public provenance.
"""
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile


def run(*args, env=None, success=True):
    result = subprocess.run(args, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    print(result.stdout, flush=True)
    assert (result.returncode == 0) == success, (args, result.returncode)
    return result.stdout.strip()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def tree(root):
    result = {}

    for path in sorted(root.rglob("*")):
        mode = stat.S_IMODE(path.lstat().st_mode)
        assert not path.is_symlink()

        if path.is_dir():
            result[path.relative_to(root).as_posix()] = dict(kind="directory", mode=mode)
        else:
            data = path.read_bytes()
            result[path.relative_to(root).as_posix()] = dict(kind="file", mode=mode, size=len(data), sha256=digest(data))

    return result


def check_approval(cli, descriptor, temp, env):
    """Check the actual image descriptor with a throwaway reviewer key."""
    key = temp / "reviewer.key"
    approval_path = temp / "approval.json"
    run(cli, "verify", "keygen", "--out", str(key), env=env)
    run(cli, "verify", "approve-backend", "--descriptor", str(descriptor), "--key", str(key), "--out", str(approval_path), env=env)
    approval = json.loads(approval_path.read_text())
    statement = json.loads(approval["statement"])
    assert statement["descriptor_sha256"] == digest(descriptor.read_bytes())
    public = temp / "reviewer.der"
    public.write_bytes(bytes.fromhex("302a300506032b6570032100" + approval["public_key"]))

    signature = temp / "signature.bin"
    signature.write_bytes(bytes.fromhex(approval["signature"]))
    message = temp / "approval-message.bin"
    domain = b"leptoskit-backend-release-approval-v1\0"
    message.write_bytes(domain + approval["statement"].encode())
    verify = ["openssl", "pkeyutl", "-verify", "-pubin", "-keyform", "DER", "-inkey", str(public), "-rawin", "-in", str(message), "-sigfile", str(signature)]
    run(*verify)
    statement["descriptor_sha256"] = "0" * 64
    message.write_bytes(domain + json.dumps(statement, separators=(",", ":")).encode())
    run(*verify, success=False)
    print("ACTUAL WEBSITE DESCRIPTOR APPROVAL VERIFIED INDEPENDENTLY; replacement digest rejected")

    return approval


root = Path(__file__).resolve().parents[2]
cli = str(Path(sys.argv[1]).resolve())
builder, runtime = sys.argv[2:4]
fixture_revision = sys.argv[4] if len(sys.argv) > 4 else "HEAD"
assert "@sha256:" in builder and "@sha256:" in runtime
fixture = root / "crates/leptoskit_cli/tests/fixtures/verified-website"
source_tree = run("git", "rev-parse", f"{fixture_revision}:crates/leptoskit_cli/tests/fixtures/verified-website")
commit = run("git", "commit-tree", "-S", source_tree, "-m", "test: committed website acceptance fixture")

env = dict(os.environ, GIT_CONFIG_COUNT="1", GIT_CONFIG_KEY_0=f"url.file://{root}.insteadOf", GIT_CONFIG_VALUE_0="https://github.com/leptoskit/leptoskit")
report = b""

for command in [["rustc", "-vV"], ["cargo", "-V"], ["leptoskit", "--version"], ["wasm-opt", "--version"]]:
    report += subprocess.check_output(["podman", "run", "--rm", "--network=none", builder, *command])
run("podman", "pull", runtime)
image = json.loads(run("podman", "image", "inspect", runtime))[0]
runtime_pin = digest(json.dumps(image["Config"], sort_keys=True, separators=(",", ":")).encode())
print(json.dumps(dict(commit=commit, builder=builder, runtime=runtime, runtime_config=runtime_pin, toolchain=report.decode())))
with tempfile.TemporaryDirectory(prefix="a04-website-acceptance-") as temporary:
    temp = Path(temporary)

    recipe = dict(repository="https://github.com/leptoskit/leptoskit", commit=commit, builder_image=builder,
                  target="aarch64-unknown-linux-gnu", package="verified-website", binary="verified-website",
                  features=[], no_default_features=False, rustflags=[], website=dict(profile="leptoskit-web-v1",
                  config_sha256=digest((fixture / "leptoskit.toml").read_bytes()), toolchain_sha256=digest(report), output_name="website_app"))
    recipe_path = temp / "recipe.json"
    recipe_path.write_text(json.dumps(recipe))
    out = temp / "build"
    run(cli, "verify", "build", "--recipe", str(recipe_path), "--out", str(out), "--engine", "podman", env=env)
    pin = digest((out / "inventory.json").read_bytes())
    common = ["--bundle", str(out / "bundle"), "--manifest", str(out / "inventory.json"), "--sha256", pin, "--engine", "podman"]
    run(cli, "verify", "reproduce", *common, env=env)
    run(cli, "verify", "reproduce", *common, "--checkout", str(root), env=env)
    tag = f"localhost/a04-website-acceptance:{os.getpid()}"
    descriptor = temp / "descriptor.json"
    package = [cli, "infra", "build-verified", *common, "--runtime-image", runtime, "--runtime-config-sha256", runtime_pin, "--tag", tag]
    try:
        run(*package, "--out", str(descriptor), env=env)
        evidence = json.loads(descriptor.read_text())
        print(descriptor.read_text())
        check_approval(cli, descriptor, temp, env)
        expected = tree(out / "bundle/artifacts/site")
        assert evidence["assets"]["reproduced"]["entries"] == expected
        assert evidence["executable_sha256"] == digest((out / "bundle/artifacts/backend").read_bytes())
        container = run("podman", "create", tag)
        try:
            extracted = temp / "extracted"
            run("podman", "cp", f"{container}:/app/dist", str(extracted))
            assert tree(extracted) == expected
            backend = temp / "extracted-backend"
            run("podman", "cp", f"{container}:/app/backend", str(backend))
            assert digest(backend.read_bytes()) == evidence["executable_sha256"]
            assert stat.S_IMODE(backend.stat().st_mode) == 0o755
            print("INDEPENDENT EXTRACTED TREE", json.dumps(tree(extracted), sort_keys=True))
            print("INDEPENDENT BACKEND SHA256", digest(backend.read_bytes()))
        finally:
            run("podman", "rm", container)
        run(*package, "--reuse-existing", "--out", str(temp / "reuse.json"), env=env)
        # Re-record a changed inventory, not just a stale-pin test: an inventory
        # proves integrity only. Fresh source reproduction must still fail.
        for relative in ["artifacts/site/pkg/website_app.js", "artifacts/site/pkg/website_app_bg.wasm", "artifacts/site/assets/app.css", "source/assets/app.css", "source/leptoskit.toml"]:
            path = out / "bundle" / relative
            original = path.read_bytes()
            path.write_bytes(original + b"\nchanged-input")
            changed_inventory = temp / "changed-inventory.json"
            run(cli, "verify", "record", "--bundle", str(out / "bundle"), "--out", str(changed_inventory), env=env)
            changed_pin = digest(changed_inventory.read_bytes())
            altered = package.copy()
            altered[altered.index("--manifest") + 1] = str(changed_inventory)
            altered[altered.index("--sha256") + 1] = changed_pin
            failed = temp / "failed.json"
            run(*altered, "--out", str(failed), env=env, success=False)
            assert not failed.exists()
            path.write_bytes(original)
            changed_inventory.unlink()

        for mutation in ["add", "delete", "mode"]:
            site = out / "bundle/artifacts/site"
            asset = site / "robots.txt"
            original = asset.read_bytes()

            if mutation == "add":
                (site / "unexpected").write_text("extra")
            elif mutation == "delete":
                asset.unlink()
            else:
                asset.chmod(0o755)
            failed = temp / "failed.json"
            run(*package, "--out", str(failed), env=env, success=False)

            assert not failed.exists()

            if mutation == "add":
                (site / "unexpected").unlink()
            else:
                asset.write_bytes(original)
                asset.chmod(0o644)
        # A self-consistent replacement report/inventory is still not the
        # pinned builder's actual toolchain report.
        report_path = out / "bundle/artifacts/website-toolchain.txt"
        original_report = report_path.read_bytes()
        build_recipe = out / "bundle/build.json"

        original_recipe = build_recipe.read_bytes()
        changed_recipe = json.loads(original_recipe)
        report_path.write_bytes(b"substituted toolchain")
        changed_recipe["website"]["toolchain_sha256"] = digest(report_path.read_bytes())
        build_recipe.write_text(json.dumps(changed_recipe))
        changed_inventory = temp / "changed-toolchain-inventory.json"
        run(cli, "verify", "record", "--bundle", str(out / "bundle"), "--out", str(changed_inventory), env=env)
        altered = package.copy()
        altered[altered.index("--manifest") + 1] = str(changed_inventory)
        altered[altered.index("--sha256") + 1] = digest(changed_inventory.read_bytes())
        failed = temp / "failed-toolchain.json"
        run(*altered, "--out", str(failed), env=env, success=False)
        assert not failed.exists()
        report_path.write_bytes(original_report)
        build_recipe.write_bytes(original_recipe)
        # An invalid runtime pin must not write success evidence either.
        altered = package.copy()
        altered[altered.index("--runtime-config-sha256") + 1] = "0" * 64
        failed = temp / "failed-runtime.json"
        run(*altered, "--out", str(failed), env=env, success=False)
        assert not failed.exists()
        altered = package.copy()
        altered[altered.index("--tag") + 1] = f"localhost/a04-missing:{os.getpid()}"
        failed = temp / "failed-engine.json"
        run(*altered, "--reuse-existing", "--out", str(failed), env=env, success=False)
        assert not failed.exists()
        # Preserve configuration and schema label, change only image asset bytes.
        wrong = temp / "wrong"
        wrong.mkdir()
        (wrong / "changed.js").write_text("false-label substitution")
        (wrong / "Dockerfile").write_text(f"FROM {tag}\nCOPY --chmod=0644 changed.js /app/dist/pkg/website_app.js\n")
        run("podman", "build", "--network=none", "--tag", tag, str(wrong))
        failed = temp / "failed-image.json"
        run(*package, "--reuse-existing", "--out", str(failed), env=env, success=False)
        assert not failed.exists()
        print("DESCRIPTOR SHA256", digest(descriptor.read_bytes()))
        print("REAL WEBSITE ACCEPTANCE PASSED; local fixture transport, no public provenance or runtime attestation claim")
    finally:
        run("podman", "image", "rm", "--force", tag)
