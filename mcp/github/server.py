#!/usr/bin/env python3
"""Local-AI reference MCP server: github (read-only).

Read-only GitHub API access over line-delimited JSON-RPC 2.0 on stdio.
No token required for public repos (rate-limited); set GITHUB_TOKEN to
raise the limit. Never writes, never pushes.

Tools:
    get_repo    — owner/repo metadata (description, stars, default branch)
    list_issues — open issues (title/number/labels, no bodies by default)
    get_readme  — decoded README (truncated)

Run:  python3 server.py --root /path/to/project   # --root unused, kept for uniformity
"""

import argparse
import base64
import json
import os
import sys
import urllib.request

API = "https://api.github.com"
TIMEOUT = 10

TOOLS = [
    {
        "name": "get_repo",
        "description": "Get public repo metadata (description, stars, language, default branch).",
        "parameters": {
            "type": "object",
            "properties": {"repo": {"type": "string", "description": "'owner/name'"}},
            "required": ["repo"],
        },
    },
    {
        "name": "list_issues",
        "description": "List open issues for a public repo (number/title/labels).",
        "parameters": {
            "type": "object",
            "properties": {
                "repo": {"type": "string"},
                "limit": {"type": "integer"},
            },
            "required": ["repo"],
        },
    },
    {
        "name": "get_readme",
        "description": "Get the decoded README of a public repo (truncated).",
        "parameters": {
            "type": "object",
            "properties": {"repo": {"type": "string"}},
            "required": ["repo"],
        },
    },
]


def api_get(path):
    req = urllib.request.Request(
        API + path,
        headers={
            "User-Agent": "Local-AI/0.1 (mcp-github)",
            "Accept": "application/vnd.github+json",
            **({"Authorization": "Bearer " + os.environ["GITHUB_TOKEN"]} if os.environ.get("GITHUB_TOKEN") else {}),
        },
    )
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
        return json.load(resp)


def valid_repo(repo):
    parts = (repo or "").split("/")
    return len(parts) == 2 and all(p and all(c.isalnum() or c in "-_." for c in p) for p in parts)


def get_repo(repo):
    if not valid_repo(repo):
        return {"error": "expected 'owner/name', got %r" % (repo,)}
    try:
        d = api_get("/repos/" + repo)
    except Exception as e:
        return {"error": "github api: %s" % (e,)}
    return {
        "repo": d.get("full_name"),
        "description": d.get("description"),
        "stars": d.get("stargazers_count"),
        "language": d.get("language"),
        "default_branch": d.get("default_branch"),
        "url": d.get("html_url"),
    }


def list_issues(repo, limit=10):
    if not valid_repo(repo):
        return {"error": "expected 'owner/name', got %r" % (repo,)}
    try:
        n = max(1, min(int(limit or 10), 30))
    except (TypeError, ValueError):
        n = 10
    try:
        items = api_get("/repos/%s/issues?state=open&per_page=%d" % (repo, n))
    except Exception as e:
        return {"error": "github api: %s" % (e,)}
    return {
        "repo": repo,
        "issues": [
            {"number": i.get("number"), "title": i.get("title"),
             "labels": [l.get("name") for l in i.get("labels", [])]}
            for i in items
            if "pull_request" not in i
        ],
    }


def get_readme(repo):
    if not valid_repo(repo):
        return {"error": "expected 'owner/name', got %r" % (repo,)}
    try:
        d = api_get("/repos/" + repo + "/readme")
        content = base64.b64decode(d.get("content", "")).decode("utf-8", "replace")
    except Exception as e:
        return {"error": "github api: %s" % (e,)}
    return {"repo": repo, "readme": content[:4000] + ("…[truncated]" if len(content) > 4000 else "")}


def handle(method, params):
    if method == "initialize":
        return {"server": "github", "version": "1"}
    if method == "tools/list":
        return {"tools": TOOLS}
    if method == "tools/call":
        name = (params or {}).get("name", "")
        args = (params or {}).get("arguments", {}) or {}
        if name == "get_repo":
            return get_repo(args.get("repo", ""))
        if name == "list_issues":
            return list_issues(args.get("repo", ""), args.get("limit", 10))
        if name == "get_readme":
            return get_readme(args.get("repo", ""))
        return {"error": "unknown tool: %s" % (name,)}
    return {"error": "unknown method: %s" % (method,)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=".")
    ap.parse_args()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError:
            continue
        if "id" not in req:
            continue
        try:
            result = handle(req.get("method", ""), req.get("params", {}))
            resp = {"jsonrpc": "2.0", "id": req["id"], "result": result}
        except Exception as e:
            resp = {"jsonrpc": "2.0", "id": req["id"],
                    "error": {"code": -32000, "message": str(e)}}
        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
