"""Execute the literal installer template; no Rust/contract validation claim."""
import concurrent.futures
import hashlib
import json
import pathlib
import platform
import re
import shutil
import stat
import subprocess
import tempfile

source_path = pathlib.Path('crates/leptoskit_cli/src/infra/schema_window.rs')
source = source_path.read_text()
template = re.search(r'r#"(set -eu\numask 077\ndirectory=.*?)"#', source, re.S).group(1)
payload = b'{"installer_probe":true}\n'
pin = hashlib.sha256(payload).hexdigest()


def quote(value):
    return "'" + str(value).replace("'", "'\\''") + "'"


def script(directory, file, old=False):
    text = template.replace('{directory}', quote(directory)).replace('{file}', quote(file)).replace('{pin}', quote(pin))
    return text.replace('ln -T -- "$temporary" "$file"', 'ln "$temporary" "$file"') if old else text


def run(directory, file, content=payload, old=False):
    return subprocess.run(['sh', '-c', script(directory, file, old)], input=content, capture_output=True)


checks = []
with tempfile.TemporaryDirectory(prefix='lk-a05-installer-') as temporary:
    directory = pathlib.Path(temporary) / "quoted ' directory"
    path = directory / (pin + '.json')
    result = run(directory, path, b'changed input')
    assert result.returncode != 0 and list(directory.iterdir()) == []
    checks.append('changed input rejected; no successful file or temporary residue')
    result = run(directory, path)
    assert result.returncode == 0, result.stderr.decode()
    assert result.stdout == pin.encode() and path.read_bytes() == payload
    assert stat.S_IMODE(path.stat().st_mode) == 0o444
    checks.append('exact bytes, pin and mode; spaces and single quotes round trip')
    inode = path.stat().st_ino
    assert run(directory, path).returncode == 0 and path.stat().st_ino == inode
    checks.append('reuse preserves existing inode')
    path.chmod(0o600)
    assert run(directory, path).returncode != 0 and stat.S_IMODE(path.stat().st_mode) == 0o600
    checks.append('wrong existing mode refused without repair')
    path.write_bytes(b'conflicting file')
    path.chmod(0o444)
    assert run(directory, path).returncode != 0 and path.read_bytes() == b'conflicting file'
    checks.append('conflicting file refused without overwrite')
    path.unlink()
    path.mkdir()
    assert run(directory, path).returncode != 0 and list(path.iterdir()) == []
    checks.append('existing directory refused without creating links inside it')
    # Required negative control: restore the previous ln command. Its failed
    # operation leaves a hard link inside the pre-existing directory.
    result = run(directory, path, old=True)
    assert result.returncode != 0 and len(list(path.iterdir())) == 1
    checks.append('negative control reproduced old directory side effect')
    shutil.rmtree(path)
    path.symlink_to('missing')
    assert run(directory, path).returncode != 0 and path.is_symlink()
    checks.append('dangling symlink refused')
    path.unlink()
    other = directory / 'other'
    other.mkdir()
    path.symlink_to(other, target_is_directory=True)
    assert run(directory, path).returncode != 0 and list(other.iterdir()) == []
    checks.append('directory symlink refused without target writes')
    path.unlink()
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        results = list(pool.map(lambda _: run(directory, path), range(2)))
    assert all(result.returncode == 0 for result in results)
    assert path.read_bytes() == payload
    assert not list(directory.glob('.incoming.*'))
    checks.append('two concurrent installs retain one correct file and clean temporaries')
    linked = pathlib.Path(temporary) / 'linked'
    linked.symlink_to(directory, target_is_directory=True)
    assert run(linked, linked / 'rejected.json').returncode != 0
    assert not (directory / 'rejected.json').exists()
    checks.append('symlink directory refused before file installation')

print(json.dumps({'status':'passed','host':platform.platform(),'source':str(source_path),
    'source_sha256':hashlib.sha256(source.encode()).hexdigest(), 'template_sha256':hashlib.sha256(template.encode()).hexdigest(),
    'checks':checks,'limits':['literal shell template only; does not exercise the Rust Window constructor','no Kamal, database, release or attestation proof']},indent=2))
