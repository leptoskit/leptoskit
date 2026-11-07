"""Curated external consumers and checks on their resolved Cargo graphs."""

import json
from pathlib import Path

PRESETS = {
    "minimal-host": ([], None),
    "browser-csr": (["web", "csr", "rpc", "db", "query"], "wasm32-unknown-unknown"),
    "browser-hydrate": (["web", "hydrate", "rpc", "db", "query"], "wasm32-unknown-unknown"),
    "native-guest": (["guest", "rpc", "db", "query"], "wasm32-unknown-unknown"),
    "server-sqlite": (["server", "rpc-live", "db-sqlite"], None),
    "server-postgres": (["server", "rpc-live", "db-postgres"], None),
    "auth-sqlite": (["server", "rpc-live", "db-sqlite", "auth"], None),
    "auth-postgres": (["server", "rpc-live", "db-postgres", "auth"], None),
}

SCREEN = '''use leptoskit::prelude::*;
pub fn screen() -> Element {
    View().class("p-4").child(Text("External packaged consumer")).into_view()
}
'''
SERVER = '''pub async fn server(url: &str) {
    let db = leptoskit::db::Db::connect(url).await.unwrap();
    let _server = leptoskit::rpc::server::Server::new(db);
    let _ = leptoskit::rpc::spec::export_if_requested;
}
'''
AUTH = '''pub fn auth_type() -> &'static str {
    std::any::type_name::<leptoskit::auth::Auth>()
}
'''
WEB = '''pub fn page() -> impl leptoskit::leptos::prelude::IntoView {
    leptoskit::web::render(screen())
}
'''
GUEST = '''pub fn mount() {
    leptoskit::guest::mount(screen);
}
'''


def write_consumer(directory, preset, packages):
    """Each fixture is its own workspace; only packaged files are patched in."""
    features, _ = PRESETS[preset]
    directory.mkdir()
    (directory / "src").mkdir()
    version = packages["leptoskit"]["version"]
    manifest = (
        f'[package]\nname = "consumer-{preset}"\nversion = "0.0.0"\nedition = "2024"\n'
        '[workspace]\nresolver = "2"\n[dependencies]\n'
        f'leptoskit = {{ version = "={version}", default-features = false, '
        f'features = {json.dumps(features)} }}\n[patch.crates-io]\n'
    )
    for name, package in sorted(packages.items()):
        manifest += f'{json.dumps(name)} = {{ path = {json.dumps(str(package["path"]))} }}\n'
    (directory / "Cargo.toml").write_text(manifest)
    source = SCREEN
    if "server" in features:
        source += SERVER
    if "auth" in features:
        source += AUTH
    if "web" in features:
        source += WEB
    if "guest" in features:
        source += GUEST
    (directory / "src/lib.rs").write_text(source)


def inspect_graph(metadata, packages, preset):
    """Reject registry fallbacks, repository paths, or server features in clients."""
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    resolved = {}
    for package in metadata["packages"]:
        name = package["name"]
        if not name.startswith("leptoskit"):
            continue
        expected = packages.get(name)
        if expected is None:
            raise ValueError(f"{preset}: {name} has no checked package archive")
        manifest = Path(package["manifest_path"]).resolve()
        if (
            package.get("source") is not None
            or package["version"] != expected["version"]
            or manifest != expected["path"] / "Cargo.toml"
        ):
            raise ValueError(f"{preset}: {name} did not resolve from its package archive")
        resolved[name] = sorted(nodes[package["id"]]["features"])
    if "leptoskit" not in resolved:
        raise ValueError(f"{preset}: no facade in resolved graph")
    expected_features, _ = PRESETS[preset]
    if not set(expected_features).issubset(resolved["leptoskit"]):
        raise ValueError(f"{preset}: requested facade features are missing")
    if "server" not in expected_features:
        forbidden = {
            "leptoskit": {"server", "db-server", "db-sqlite", "db-postgres"},
            "leptoskit_db": {"server", "sqlite", "postgres"},
            "leptoskit_rpc": {"server", "sqlite", "postgres"},
        }
        for name, features in forbidden.items():
            if features.intersection(resolved.get(name, [])):
                raise ValueError(f"{preset}: server features leaked into {name}")
    return resolved
