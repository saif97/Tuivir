#!/usr/bin/env python3
"""Small stateful GitHub boundary fake; never contacts GitHub."""

import json
import hashlib
import base64
import os
import sys
from pathlib import Path

path = Path(os.environ["RELEASE_GITHUB_STATE"])
state = json.loads(path.read_text())
args = sys.argv[1:]
if args[0] == "api" and state.get("api_error"):
    sys.exit(state["api_error"])
if args[0] == "release":
    operation, tag = args[1:3]
    if operation == "create":
        if state.get("release"):
            sys.exit("release already exists")
        state["release"] = {"tag_name": tag, "draft": True, "prerelease": False,
                            "immutable": False, "assets": []}
    elif operation == "upload":
        if not state["release"]["draft"]:
            sys.exit("cannot overwrite a published release")
        files = args[3:args.index("--repo")]
        state["release"]["assets"] = [
            {"name": Path(file).name, "digest": "sha256:" + hashlib.sha256(Path(file).read_bytes()).hexdigest()}
            for file in files
        ]
        state["asset_files"] = {Path(file).name: base64.b64encode(Path(file).read_bytes()).decode()
                                for file in files}
        if state.pop("fail_upload_once", False):
            state["release"]["assets"] = state["release"]["assets"][:1]
            path.write_text(json.dumps(state))
            sys.exit("simulated upload failure")
    elif operation == "edit":
        if len(state["release"]["assets"]) != 4:
            sys.exit("refusing to publish incomplete assets")
        state["release"]["draft"] = False
        state["release"]["immutable"] = state.get("immutability_enabled", True)
        state["marked_latest"] = "--latest=true" in args
    elif operation == "download":
        destination = Path(args[args.index("--dir") + 1])
        destination.mkdir(parents=True)
        for name, content in state["asset_files"].items():
            (destination / name).write_bytes(base64.b64decode(content))
        sys.exit(0)
    else:
        sys.exit(f"unexpected release operation: {operation}")
    path.write_text(json.dumps(state))
    sys.exit(0)
if args[0] == "api" and args[1].startswith("repos/saif97/Tuivir/releases"):
    if "/releases?" in args[1]:
        print(json.dumps([[state["release"]] if state.get("release") else []]))
        sys.exit(0)
    release = state.get("latest") if args[1].endswith("/latest") else state.get("release")
    # GitHub's tag endpoint finds published releases; drafts need a list lookup.
    if not release or release.get("draft"):
        sys.exit("gh: Not Found (HTTP 404)")
    print(json.dumps(release))
    sys.exit(0)
if args[:2] != ["api", "repos/saif97/homebrew-tap/contents/Formula/tuivir.rb"]:
    sys.exit(f"unexpected GitHub request: {args}")
method = args[args.index("--method") + 1] if "--method" in args else "GET"
if method == "GET":
    print(json.dumps({"content": state["content"], "sha": str(state["revision"])}))
elif method == "PUT":
    data = json.load(sys.stdin)
    if data["sha"] != str(state["revision"]):
        sys.exit("gh: Conflict (HTTP 409)")
    state["content"] = data["content"]
    state["revision"] += 1
    path.write_text(json.dumps(state))
    print(json.dumps({"commit": {"sha": str(state["revision"])}}))
else:
    sys.exit(f"unexpected method: {method}")
