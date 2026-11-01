"""Thin, bounded protocol over installed Colibri catalogue/download APIs.

No setup flow, engine build, conversion, subprocess launch or inference belongs
here. Colibri owns transfer, partial files, resumption and digest verification.
"""
import hashlib
import importlib
import json
import os
from pathlib import Path, PurePosixPath
import signal
import shutil
import stat
import sys
import tempfile
import time
import urllib.parse
import urllib.request

LIMIT = 2 * 1024 * 1024
BASE = "https://huggingface.co"


def require(condition):
    if not condition:
        raise ValueError("unsupported acquisition contract")


def read_json(path):
    fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as handle:
        require(stat.S_ISREG(os.fstat(handle.fileno()).st_mode))
        data = handle.read(LIMIT + 1)
    require(len(data) <= LIMIT)
    return json.loads(data)


def atomic(path, value):
    path = Path(path)
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as handle:
        temporary = handle.name
        json.dump(value, handle, separators=(",", ":"))
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temporary, path)


def safe_url(value):
    url = urllib.parse.urlsplit(value)
    host = url.hostname or ""
    require(url.scheme == "https" and url.port in (None, 443)
            and not url.username and not url.password
            and (host == "huggingface.co" or host.endswith(".huggingface.co")
                 or host.endswith(".hf.co")))


class Redirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, url):
        safe_url(url)
        return super().redirect_request(request, fp, code, message, headers, url)


OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}), Redirect())
METADATA_REMAINING = [4 * LIMIT]


class MetadataResponse:
    def __init__(self, response):
        self.response = response
        self.headers = response.headers

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.response.close()

    def read(self):
        data = self.response.read(LIMIT + 1)
        require(len(data) <= LIMIT)
        METADATA_REMAINING[0] -= len(data)
        require(METADATA_REMAINING[0] >= 0)
        return data


def open_remote(request, timeout=60):
    url = request.full_url if hasattr(request, "full_url") else request
    safe_url(url)
    response = OPENER.open(request, timeout=min(timeout, 60))
    if urllib.parse.urlsplit(url).path.startswith("/api/"):
        return MetadataResponse(response)
    return response


def identifier(value):
    return (isinstance(value, str) and 0 < len(value) <= 160
            and all(c.isascii() and (c.isalnum() or c in "-_.") for c in value)
            and value not in (".", ".."))


def entry_data(entry):
    require(identifier(entry.id) and identifier(entry.family)
            and isinstance(entry.name, str) and len(entry.name) <= 256)
    require(len(entry.repo.split("/")) == 2
            and all(identifier(part) for part in entry.repo.split("/")))
    artifact = families.family_by_id(entry.family).engine_artifact
    require(identifier(artifact))
    available = (module_dir / artifact).is_file() and os.access(module_dir / artifact, os.X_OK)
    return {"id": entry.id, "family": entry.family, "name": entry.name, "engine_artifact": artifact, "engine_available": available,
            "repo": entry.repo, "conversion_required": bool(entry.post_install),
            "license": str(entry.license_note or "")[:512]}


def files_valid(files):
    require(isinstance(files, list) and 0 < len(files) <= 4096)
    names = set()
    for spec in files:
        name = spec["path"]
        require(isinstance(name, str) and 0 < len(name) <= 512
                and "\\" not in name and not any(ord(c) < 32 for c in name))
        parts = PurePosixPath(name).parts
        require(parts and not PurePosixPath(name).is_absolute()
                and all(p not in (".", "..") for p in parts)
                and str(PurePosixPath(name)) == name)
        require(name not in names and name + ".part" not in names
                and name not in (download.MANIFEST, download.MANIFEST + ".tmp"))
        names.update((name, name + ".part"))
        require(isinstance(spec["size"], int) and not isinstance(spec["size"], bool)
                and 0 <= spec["size"] <= 8 * 1024**4)
        digest = spec.get("sha256") or spec.get("git_oid")
        require(isinstance(digest, str) and len(digest) in (40, 64)
                and all(c in "0123456789abcdefABCDEF" for c in digest))


def destination_valid(root, files):
    require(root.is_absolute() and len(str(root)) <= 4096)
    for name in (download.MANIFEST, download.MANIFEST + ".tmp"):
        candidate = root / name
        require(not candidate.is_symlink())
        if candidate.exists():
            require(candidate.is_file())
    # Do not let native resumption overwrite a symlink or unrelated existing file.
    for spec in files:
        target = Path(download.safe_join(str(root), spec["path"]))
        for candidate in (target, Path(str(target) + ".part")):
            for parent in (candidate, *candidate.parents):
                if parent == root.parent:
                    break
                require(not parent.is_symlink())
            if candidate.exists():
                require(candidate.is_file())


def complete_snapshot(root, manifest):
    files = manifest.get("files")
    require(manifest.get("complete") is True and isinstance(files, list)
            and 0 < len(files) <= 4096)
    for spec in files:
        name = spec["path"]
        require(isinstance(name, str) and 0 < len(name) <= 512)
        target = Path(download.safe_join(str(root), name))
        require(isinstance(spec["size"], int) and 0 <= spec["size"] <= 8 * 1024**4
                and target.is_file() and target.stat().st_size == spec["size"])
    return True


def plan(request):
    entry = catalogue.by_id(request["model"])
    result = entry_data(entry)
    revision = request.get("revision") or entry.revision
    require(isinstance(revision, str) and len(revision) <= 160)
    metadata_url = BASE + "/api/models/" + entry.repo + "/revision/" + urllib.parse.quote(revision, safe="")
    with open_remote(metadata_url) as response:
        metadata = json.loads(response.read())
    revision = metadata["sha"]
    require(len(revision) == 40 and all(c in "0123456789abcdef" for c in revision))
    files = download.list_repo_files(entry.repo, revision, include=entry.include,
                                    exclude=entry.exclude, base=BASE, opener=open_remote)
    files_valid(files)
    files = [{"path": f["path"], "size": f["size"], "sha256": f.get("sha256") or "",
              "git_oid": f.get("git_oid") or ""} for f in files]
    root = Path(request["directory"])
    destination_valid(root, files)
    total = sum(spec["size"] for spec in files)
    require(0 < total <= 8 * 1024**4)
    license_value = (metadata.get("cardData") or {}).get("license")
    if isinstance(license_value, str):
        result["license"] = license_value[:512]
    result.update(revision=revision, directory=str(root), files=files, total=total,
                  present=min(total, download.bytes_present(str(root), files)))
    result["fingerprint"] = fingerprint(result)
    return result


def fingerprint(value):
    value = {k: v for k, v in value.items() if k not in ("fingerprint", "present", "engine_available")}
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


class Cancelled(Exception):
    pass


def acquire(value, state_path, verify_only=False):
    require(value["fingerprint"] == fingerprint(value))
    files = value["files"]
    files_valid(files)
    root = Path(value["directory"])
    destination_valid(root, files)
    if not verify_only:
        ancestor = root
        while not ancestor.exists():
            ancestor = ancestor.parent
        require(shutil.disk_usage(ancestor).free >= max(0, value["total"] - download.bytes_present(str(root), files)) + 16 * 1024 * 1024)
    existing_manifest = root / download.MANIFEST
    if existing_manifest.exists():
        existing = read_json(existing_manifest)
        require(existing.get("repo") == value["repo"]
                and (verify_only or existing.get("revision") == value["revision"]))
    state = {"status": "verifying", "done": 0, "total": value["total"],
             "model": value["id"], "fingerprint": value["fingerprint"], "error": ""}
    cancelled = [False]
    signal.signal(signal.SIGTERM, lambda *_: cancelled.__setitem__(0, True))
    last = [0.0]

    def publish(event=None, force=False):
        if cancelled[0]:
            raise Cancelled()
        if event:
            state.update(done=int(event["done"]), total=int(event["total"]))
        if force or time.monotonic() - last[0] >= 0.25:
            atomic(state_path, state)
            last[0] = time.monotonic()

    publish(force=True)
    try:
        for spec in files:
            target = download.safe_join(str(root), spec["path"])
            if os.path.exists(target):
                require(download.verify_file(target, spec))
            elif verify_only:
                raise ValueError("missing acquired file")
            publish()
        if not verify_only:
            state["status"] = "downloading"
            download.download_repo(value["repo"], value["revision"], str(root), files,
                                   base=BASE, opener=open_remote, progress=publish, verify=True)
        state["status"] = "verifying"
        publish(force=True)
        for spec in files:
            require(download.verify_file(download.safe_join(str(root), spec["path"]), spec))
            publish()
        state.update(status="complete", done=value["total"])
    except Cancelled:
        state["status"] = "paused"
    except Exception:
        state.update(status="failed", error="Colibri native acquisition or integrity verification failed")
    atomic(state_path, state)
    return state["status"] == "complete"


value = None
try:
    module_dir = Path(sys.argv[1])
    require(module_dir.is_absolute())
    mode = sys.argv[2]
    if mode in ("download", "verify"):
        value = read_json(sys.argv[3])
        require(value["fingerprint"] == fingerprint(value))
    sys.path.insert(0, str(module_dir))
    catalogue = importlib.import_module("setup_catalog")
    download = importlib.import_module("setup_download")
    families = importlib.import_module("family_registry")
    if mode in ("download", "verify"):
        sys.exit(0 if acquire(value, sys.argv[4], mode == "verify") else 1)
    data = sys.stdin.buffer.read(LIMIT + 1)
    require(len(data) <= LIMIT)
    request = json.loads(data)
    if mode == "catalogue":
        result = [entry_data(entry) for entry in catalogue.catalog() if entry.modality == "text"]
        require(len(result) <= 128)
        for item in result:
            root = Path.home() / "colibri-models" / item["id"]
            if request.get("current_model") == item["id"]:
                root = Path(request["current_directory"])
            item["directory"] = str(root)
            try:
                manifest = root / download.MANIFEST
                if manifest.exists():
                    value = read_json(manifest)
                    item["downloaded"] = complete_snapshot(root, value)
                else:
                    item["downloaded"] = False
            except Exception:
                item["downloaded"] = False
    elif mode == "plan":
        result = plan(request)
    else:
        raise ValueError("unsupported worker operation")
    encoded = json.dumps(result, separators=(",", ":"))
    require(len(encoded.encode()) <= LIMIT)
    print(encoded)
except Exception:
    # Import and pre-transfer validation errors must not leave a transfer stuck
    # in "starting" after the native host exits. Only publish for a valid scope.
    try:
        if value is not None and value["fingerprint"] == fingerprint(value):
            atomic(sys.argv[4], {"status": "failed", "done": 0, "total": value["total"],
                                "model": value["id"], "fingerprint": value["fingerprint"],
                                "error": "Colibri native acquisition or integrity verification failed"})
    except Exception:
        pass
    # Native exceptions can contain URLs, tokens or local paths. Never forward them.
    print("Colibri native API or acquisition contract is unavailable", file=sys.stderr)
    sys.exit(1)
