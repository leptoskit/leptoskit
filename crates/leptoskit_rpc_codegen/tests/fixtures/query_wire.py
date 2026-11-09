"""Exercise generated public methods and request preparation without a network."""
import json
import urllib.parse

from wire.client import Client
from wire.live import _present, UNSET

assert _present({"note": None, "missing": UNSET, "items": []}) == {"note": None, "items": []}


class Observe(Client):
    def _call(self, method, path, encoding, payload, idempotency_key=None):
        url, _, body = self._prepare(method, path, encoding, payload, idempotency_key)
        return {
            "query": urllib.parse.urlsplit(url).query,
            "body": body.decode() if body is not None else None,
        }


client = Observe()
values = {"items": [], "labels": {}, "note": None, "enabled": False, "count": 0, "text": ""}
print(json.dumps([
    client.probe(**values),
    client.probe(**values, fallback=None),
    client.probe(**values, fallback=""),
    client.probe(**{**values, "items": [None, "a +&é", ""], "labels": {"key +&é": None, "empty": ""}, "text": "a +&é"}),
]))
