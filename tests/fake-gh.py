import json, os, subprocess, sys, uuid

D = os.environ["FAKE_GH_DIR"]
a = sys.argv[1:]


def ld(n, d):
    p = os.path.join(D, n)
    return json.load(open(p)) if os.path.exists(p) else d


def sv(n, v):
    json.dump(v, open(os.path.join(D, n), "w"))


def out(v):
    print(json.dumps(v))
    sys.exit(0)


def opt(k):
    return a[a.index(k) + 1] if k in a else None


# Multiple CLI processes can run concurrently. Publish each complete record
# independently; append on Windows is not a reliable interprocess lock.
calls = os.path.join(D, "calls")
os.makedirs(calls, exist_ok=True)
call_id = uuid.uuid4().hex
temporary = os.path.join(calls, call_id + ".tmp")
with open(temporary, "x", encoding="utf-8") as record:
    json.dump(a, record)
os.replace(temporary, os.path.join(calls, call_id + ".json"))
R = os.environ.get("FAKE_GH_REPO", "o/r")


def rest_pr(pr):
    # Resolve the actual pushed SHA from the isolated bare Git fixture.
    branch = pr["head"]
    remote = os.path.join(os.path.dirname(D), "remotes", R + ".git")
    sha = subprocess.check_output(
        ["git", "--git-dir", remote, "rev-parse", "refs/heads/" + branch],
        text=True,
    ).strip()
    return {
        "id": pr["number"],
        "number": pr["number"],
        "state": pr.get("state", "open"),
        "html_url": pr["url"],
        "title": pr["title"],
        "body": pr["body"],
        "head": {"repo": {"full_name": R}, "ref": branch, "sha": sha},
        "base": {"repo": {"full_name": R}, "ref": pr["base"]},
    }

if a[:2] == ["auth", "token"]:
    print("fake-token")
    sys.exit(0)
if a[0] == "api":
    p = [x for x in a[1:] if not x.startswith("-") and x not in ("Accept: application/vnd.github+json", "default_branch_only=true", ".rate.remaining", ".login", "{s: .sha}")]
    p = p[0] if p else ""
    if p == "user":
        if "--jq" in a:
            print("tester")
            sys.exit(0)
        out({"login": "tester", "id": 1, "name": "Tester"})
    if p == "rate_limit":
        print(5000)
        sys.exit(0)
    if p == f"repos/{R}":
        out({"full_name": R, "default_branch": "main", "private": os.environ.get("FAKE_GH_PRIVATE") == "1", "fork": False, "archived": False, "permissions": {"push": True}})
    if p.startswith(f"repos/{R}/issues?"):
        out([{**i, "state": "open", "html_url": i["url"], "user": i.get("author"),
              "created_at": i.get("createdAt", i["updatedAt"]), "updated_at": i["updatedAt"]}
             for i in ld("issues.json", [])])
    if p.startswith(f"repos/{R}/pulls?state=open&"):
        out([rest_pr(pr) for pr in ld("prs.json", []) if pr.get("state", "open") == "open"])
    if p.startswith(f"repos/{R}/pulls/"):
        # Exercise a transient verification failure after successful creation.
        # Only the private fixture opts in; record the fault before returning.
        marker = os.path.join(D, "receipt-fault.json")
        if os.environ.get("FAKE_GH_FAIL_RECEIPT_ONCE") == "1" and not os.path.exists(marker):
            # Move main to a conflicting commit after the original PR exists.
            # Receipt recovery must not rebase or republish that PR.
            seed = os.path.join(os.path.dirname(D), "seed")
            remote = os.path.join(os.path.dirname(D), "remotes", R + ".git")
            with open(os.path.join(seed, "add.sh"), "w") as changed:
                changed.write("add() { echo $(( $1 * $2 )); }\n")
            subprocess.check_call(["git", "-C", seed, "add", "add.sh"])
            subprocess.check_call(
                ["git", "-C", seed, "-c", "user.name=fixture", "-c",
                 "user.email=fixture@example.test", "commit", "-qm", "conflicting main"]
            )
            subprocess.check_call(["git", "-C", seed, "push", "-q", remote, "main"])
            sv("receipt-fault.json", {"endpoint": p, "main_advanced": True})
            sys.stderr.write("HTTP 503: injected receipt verification failure")
            sys.exit(1)
        number = p.rsplit("/", 1)[-1]
        for pr in ld("prs.json", []):
            if str(pr["number"]) == number:
                out(rest_pr(pr))
        sys.stderr.write("HTTP 404: Not Found")
        sys.exit(1)
    if "/timeline" in p or "/comments" in p or "/reviews" in p:
        out([[]] if "--slurp" in a else [])
    if "/contents/" in p:
        sys.stderr.write("HTTP 404: Not Found")
        sys.exit(1)
    sys.stderr.write("fake gh: unknown api " + p)
    sys.exit(1)
if a[:2] == ["issue", "list"]:
    out(ld("issues.json", []))
if a[:2] == ["issue", "view"]:
    n = int(a[2])
    for i in ld("issues.json", []):
        if i["number"] == n:
            out({**i, "state": "OPEN", "comments": [], "assignees": []})
    sys.exit(1)
if a[:2] == ["pr", "list"]:
    if opt("--head"):
        out([p for p in ld("prs.json", []) if p["head"] == opt("--head")])
    out([])
if a[:2] == ["pr", "create"]:
    if opt("--body-file") != "-":
        sys.stderr.write("expected PR body on stdin")
        sys.exit(1)
    body = sys.stdin.buffer.read().decode("utf-8")
    ps = ld("prs.json", [])
    n = 100 + len(ps)
    ps.append({"number": n, "url": f"https://github.com/{R}/pull/{n}", "head": opt("--head"), "base": opt("--base"), "title": opt("--title"), "body": body, "headRepositoryOwner": {"login": "tester"}})
    sv("prs.json", ps)
    print(f"https://github.com/{R}/pull/{n}")
    sys.exit(0)
if a[:2] == ["pr", "view"]:
    out({"state": "OPEN", "mergedAt": None, "updatedAt": "2026-01-01T00:00:00Z", "title": "x", "url": "u", "mergeable": "MERGEABLE", "headRefName": "b", "headRepositoryOwner": {"login": "tester"}, "headRepository": {"name": "r"}, "baseRefName": "main"})
sys.stderr.write("fake gh: unknown " + " ".join(a))
sys.exit(1)
