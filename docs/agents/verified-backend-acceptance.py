"""Real Podman acceptance. Run after committing with the CLI path as argument.

Uses a signed commit of the committed fixture tree and invocation-only local
Git transport mapping. Tests full reproduction mechanics, not public provenance.
No checkout, registry publication, deployment, or shared Git config changes.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def run(*args, env=None, success=True):
    result = subprocess.run(args, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    print(result.stdout, flush=True)
    assert (result.returncode == 0) == success, args
    return result.stdout.strip()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def check_rpc_compatibility(package, temp, env):
    """Compare the real image export, including rejection after a baseline rehash.

    The tiny fixture declares an empty API. This tests the packaging boundary;
    nonempty function/channel comparisons and operator policy have separate tests.
    """
    baseline = temp / "rpc-baseline.json"
    baseline.write_text(json.dumps(dict(leptoskit_rpc_spec=3, title="published", version="1", functions=[])))
    baseline_bytes = baseline.read_bytes()
    pin = digest(baseline_bytes)
    checked = package + ["--rpc-baseline", str(baseline), "--rpc-baseline-sha256", pin]
    descriptor = temp / "rpc-checked.json"
    run(*checked, "--out", str(descriptor), env=env)
    evidence = json.loads(descriptor.read_text())
    assert evidence["format_version"] == 3
    contract = evidence["rpc_contract"]
    compatibility = contract["compatibility"]["checked"]
    assert compatibility["algorithm"] == "leptoskit-rpc-structural-v1"
    assert digest(compatibility["specification"].encode()) == contract["sha256"]
    assert compatibility["baselines"] == [dict(sha256=pin, specification=baseline_bytes.decode())]
    run(*checked, "--reuse-existing", "--out", str(temp / "rpc-reuse.json"), env=env)

    # Changing the file cannot keep its original independent pin.
    baseline.write_bytes(baseline_bytes + b" ")
    failed = temp / "rpc-replaced.json"
    run(*checked, "--out", str(failed), env=env, success=False)
    assert not failed.exists()

    # A previously supported endpoint is absent from this actual executable.
    # Even a self-consistent baseline pin must fail the comparison.
    prior = json.loads((root / "crates/leptoskit_cli/tests/fixtures/fns_spec.json").read_text())
    baseline.write_text(json.dumps(prior))
    incompatible = checked.copy()
    incompatible[incompatible.index("--rpc-baseline-sha256") + 1] = digest(baseline.read_bytes())
    failed = temp / "rpc-incompatible.json"
    run(*incompatible, "--reuse-existing", "--out", str(failed), env=env, success=False)
    assert not failed.exists()
    baseline.write_bytes(baseline_bytes)
    failed = temp / "rpc-no-api.json"
    run(*checked, "--no-rpc", "--out", str(failed), env=env, success=False)
    assert not failed.exists()

    # The exact checked contract bytes enter the existing approval domain.
    key = temp / "rpc-reviewer.key"
    approval_path = temp / "rpc-approval.json"
    run(cli, "verify", "keygen", "--out", str(key), env=env)
    run(cli, "verify", "approve-backend", "--descriptor", str(descriptor), "--key", str(key), "--out", str(approval_path), env=env)
    approval = json.loads(approval_path.read_text())
    statement = json.loads(approval["statement"])
    assert statement["descriptor_sha256"] == digest(descriptor.read_bytes())
    public = temp / "rpc-reviewer.der"
    public.write_bytes(bytes.fromhex("302a300506032b6570032100" + approval["public_key"]))
    signature = temp / "rpc-signature.bin"
    signature.write_bytes(bytes.fromhex(approval["signature"]))
    message = temp / "rpc-approval-message.bin"
    domain = b"leptoskit-backend-release-approval-v1\0"
    message.write_bytes(domain + approval["statement"].encode())
    verify = ["openssl", "pkeyutl", "-verify", "-pubin", "-keyform", "DER", "-inkey", str(public), "-rawin", "-in", str(message), "-sigfile", str(signature)]
    run(*verify)
    statement["descriptor_sha256"] = "0" * 64
    message.write_bytes(domain + json.dumps(statement, separators=(",", ":")).encode())
    run(*verify, success=False)
    print("REAL RPC PACKAGING ACCEPTANCE PASSED; structural declarations only")


root = Path(__file__).resolve().parents[2]
cli = str(Path(sys.argv[1]).resolve())
tree = run("git", "rev-parse", "HEAD:crates/leptoskit_cli/tests/fixtures/verified-backend")
commit = run("git", "commit-tree", "-S", tree, "-m", "test: committed backend acceptance fixture")
env = dict(os.environ, GIT_CONFIG_COUNT="1", GIT_CONFIG_KEY_0=f"url.file://{root}.insteadOf", GIT_CONFIG_VALUE_0="https://github.com/leptoskit/leptoskit")
images = json.loads(run("podman", "image", "inspect", "rust:1.85-alpine", "alpine:3.21"))
builder, runtime = [item["RepoDigests"][0] for item in images]
runtime_pin = digest(json.dumps(images[1]["Config"], sort_keys=True, separators=(",", ":")).encode())
print(json.dumps(dict(commit=commit, builder=builder, runtime=runtime, runtime_config=runtime_pin)))
with tempfile.TemporaryDirectory(prefix="a04-acceptance-") as temp:
    temp = Path(temp)
    recipe = temp / "recipe.json"
    recipe.write_text(json.dumps(dict(repository="https://github.com/leptoskit/leptoskit", commit=commit, builder_image=builder, target="aarch64-unknown-linux-musl", package="verified-backend", binary="verified-backend", features=[], no_default_features=False, rustflags=[])))
    out = temp / "build"
    run(cli, "verify", "build", "--recipe", str(recipe), "--out", str(out), "--engine", "podman", env=env)
    pin = digest((out / "inventory.json").read_bytes())
    common = ["--bundle", str(out / "bundle"), "--manifest", str(out / "inventory.json"), "--sha256", pin, "--engine", "podman"]
    run(cli, "verify", "reproduce", *common, env=env)
    tag = f"localhost/a04-acceptance:{os.getpid()}"
    descriptor = temp / "descriptor.json"
    package = [cli, "infra", "build-verified", *common, "--runtime-image", runtime, "--runtime-config-sha256", runtime_pin, "--tag", tag]
    try:
        run(*package, "--out", str(descriptor), env=env)
        print(descriptor.read_text())
        evidence = json.loads(descriptor.read_text())
        assert evidence["executable_sha256"] == digest((out / "bundle/artifacts/backend").read_bytes())
        assert evidence["engine_image_id"].startswith("sha256:")
        container = run("podman", "create", tag)
        try:
            extracted = temp / "extracted-backend"
            run("podman", "cp", f"{container}:/app/backend", str(extracted))
            assert digest(extracted.read_bytes()) == evidence["executable_sha256"]
            print("independent extracted executable SHA-256", digest(extracted.read_bytes()))
        finally:
            run("podman", "rm", container)
        run(*package, "--reuse-existing", "--out", str(temp / "reuse.json"), env=env)
        check_rpc_compatibility(package, temp, env)
        for relative in ["artifacts/backend", "source/src/main.rs", "source/migrations/registry.txt", "source/leptoskit.toml", "build.json"]:
            path = out / "bundle" / relative
            original = path.read_bytes()
            path.write_bytes(original + b"substitution")
            failed = temp / "failed.json"
            run(*package, "--out", str(failed), env=env, success=False)
            assert not failed.exists(), "failure wrote success evidence"
            path.write_bytes(original)
        failed = temp / "failed-config.json"
        changed = package.copy()
        changed[changed.index("--runtime-config-sha256") + 1] = "0" * 64
        run(*changed, "--reuse-existing", "--out", str(failed), env=env, success=False)
        assert not failed.exists()
        # Preserve the schema label and entrypoint but replace the actual bytes.
        wrong = temp / "wrong"
        wrong.mkdir()
        (wrong / "backend").write_bytes(b"wrong executable")
        (wrong / "Dockerfile").write_text(f"FROM {tag}\nCOPY --chmod=0755 backend /app/backend\n")
        run("podman", "build", "--network=none", "--tag", tag, str(wrong))
        failed = temp / "failed-bytes.json"
        run(*package, "--reuse-existing", "--out", str(failed), env=env, success=False)
        assert not failed.exists()
        print("REAL ENGINE ACCEPTANCE PASSED; local fixture transport, no public provenance claim")
    finally:
        run("podman", "image", "rm", "--force", tag)
