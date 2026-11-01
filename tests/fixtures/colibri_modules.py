"""Native API test double. Never imports or installs Colibri or contacts a Hub."""
import hashlib
import io
import json
import os
from pathlib import Path
from types import SimpleNamespace
import sys

ROOT = Path(__file__).parent
MANIFEST = ".colibri-download.json"


def record(value):
    with (ROOT / "calls.jsonl").open("a") as handle:
        handle.write(json.dumps(value) + "\n")


def fixture():
    return json.loads((ROOT / "fixture.json").read_text())


class Response(io.BytesIO):
    headers = {}


class Opener:
    def open(self, request, timeout=None):
        url = request.full_url if hasattr(request, "full_url") else request
        record({"metadata": url})
        return Response(json.dumps({"sha": "a" * 40, "cardData": {"license": "mit"}}).encode())


sys.modules["__main__"].OPENER = Opener()


def catalogue_entry():
    return SimpleNamespace(id="example", family="glm", name="Example model",
                           repo="example/model", revision="main", post_install=(),
                           license_note="mit", modality="text", include=(), exclude=())


def catalog():
    return (catalogue_entry(),)


def by_id(identity):
    if identity != "example":
        raise ValueError("private native diagnostic")
    return catalogue_entry()


def family_by_id(identity):
    return SimpleNamespace(engine_artifact="colibri")


def list_repo_files(repo, revision, **kwargs):
    record({"list": repo, "revision": revision})
    data = fixture()
    if data.get("invalid"):
        return [{"path": "../private-escape", "size": 8, "sha256": "b" * 64}]
    return [{"path": name, "size": len(content.encode()),
             "sha256": hashlib.sha256(content.encode()).hexdigest(), "git_oid": None}
            for name, content in data["files"].items()]


def safe_join(root, name):
    return str(Path(root) / name)


def bytes_present(root, files):
    return sum(min(Path(root, spec["path"]).stat().st_size, spec["size"])
               if Path(root, spec["path"]).is_file() else 0 for spec in files)


def verify_file(path, spec):
    record({"verify": Path(path).name})
    return (Path(path).is_file() and Path(path).stat().st_size == spec["size"]
            and hashlib.sha256(Path(path).read_bytes()).hexdigest() == spec["sha256"])


def download_repo(repo, revision, root, files, progress=None, **kwargs):
    record({"download": repo, "revision": revision})
    Path(root).mkdir(parents=True, exist_ok=True)
    data = fixture()["files"]
    done = 0
    for spec in files:
        target = Path(root, spec["path"])
        if not target.exists():
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(data[spec["path"]])
            record({"write": spec["path"]})
        done += spec["size"]
        if progress:
            progress({"done": done, "total": sum(s["size"] for s in files)})
    Path(root, MANIFEST).write_text(json.dumps({"repo": repo, "revision": revision,
                                             "complete": True, "files": files}))


def hf_token():
    raise AssertionError("Credentials must not be read during public acquisition")


def is_complete(root):
    manifest = Path(root, MANIFEST)
    if not manifest.is_file():
        return False
    data = json.loads(manifest.read_text())
    return data.get("complete", False) and all(Path(root, f["path"]).is_file()
                                              and Path(root, f["path"]).stat().st_size == f["size"]
                                              for f in data["files"])
