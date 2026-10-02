#!/usr/bin/env python3
"""Local-AI reference MCP server: filesystem (read-only).

Exposes the attached project root over line-delimited JSON-RPC 2.0 on stdio:

    {"jsonrpc":"2.0","id":1,"method":"initialize","params":{...}}
    {"jsonrpc":"2.0","id":2,"method":"tools/list"}
    {"jsonrpc":"2.0","id":3,"method":"tools/call",
     "params":{"name":"read_file","arguments":{"path":"src/main.rs"}}}

Tools (read-only first — no write/exec here):
    list_files  — recursive inventory (ignores .git/node_modules/target/…)
    read_file   — jailed read (relative paths only, `..` and absolute refused)

Run:  python3 server.py --root /path/to/project
"""

import argparse
import json
import os
import sys

IGNORED_DIRS = {
    "node_modules", ".git", "target", "dist", "build", ".next",
    ".idea", ".vscode", "__pycache__", ".venv", "venv", ".cache",
    "coverage", ".fastembed_cache", ".huggingface",
}

TOOLS = [
    {
        "name": "list_files",
        "description": "List all files under the project root (authoritative inventory).",
        "parameters": {"type": "object", "properties": {}, "required": []},
    },
    {
        "name": "read_file",
        "description": "Read a project file. Path must be repo-relative (e.g. 'src/main.rs').",
        "parameters": {
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
        },
    },
]


def safe_join(root, rel):
    if os.path.isabs(rel) or ".." in rel.split(os.sep) and ".." in rel.replace("\\", "/").split("/"):
        return None
    # Reject absolute, drive-letter and parent escapes up front.
    norm = rel.replace("\\", "/")
    if norm.startswith("/") or ".." in norm.split("/"):
        return None
    if len(norm) >= 2 and norm[1] == ":":
        return None
    full = os.path.realpath(os.path.join(root, *norm.split("/")))
    if full != root and not full.startswith(root + os.sep):
        return None
    return full


def list_files(root):
    out = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in IGNORED_DIRS]
        for fn in filenames:
            full = os.path.join(dirpath, fn)
            rel = os.path.relpath(full, root).replace(os.sep, "/")
            try:
                size = os.path.getsize(full)
            except OSError:
                continue
            out.append({"path": rel, "size": size})
    out.sort(key=lambda e: e["path"])
    return {"files": out, "total": len(out)}


def read_file(root, path):
    full = safe_join(root, path or "")
    if full is None:
        return {"error": "refusing path outside project root: %r" % (path,)}
    if not os.path.isfile(full):
        return {"error": "not found: %s" % (path,)}
    try:
        with open(full, "r", encoding="utf-8", errors="replace") as f:
            content = f.read(8000)
    except OSError as e:
        return {"error": str(e)}
    return {"path": path, "content": content}


def handle(method, params, root):
    if method == "initialize":
        return {"server": "filesystem", "version": "1", "root": root}
    if method == "tools/list":
        return {"tools": TOOLS}
    if method == "tools/call":
        name = (params or {}).get("name", "")
        args = (params or {}).get("arguments", {}) or {}
        if name == "list_files":
            return list_files(root)
        if name == "read_file":
            return read_file(root, args.get("path", ""))
        return {"error": "unknown tool: %s" % (name,)}
    return {"error": "unknown method: %s" % (method,)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=".")
    args = ap.parse_args()
    root = os.path.realpath(args.root)
    stdin = sys.stdin
    for line in stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError:
            continue  # ignore non-JSON stdin noise
        if "id" not in req:
            continue  # notification — no response
        try:
            result = handle(req.get("method", ""), req.get("params", {}), root)
            resp = {"jsonrpc": "2.0", "id": req["id"], "result": result}
        except Exception as e:  # never let one call kill the server
            resp = {"jsonrpc": "2.0", "id": req["id"],
                    "error": {"code": -32000, "message": str(e)}}
        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
