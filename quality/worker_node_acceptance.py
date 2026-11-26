"""Additional F01 checks on the disposable Docker worker created by CI."""
import os
from pathlib import Path
import re
import shlex
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def shell_constant(path, name):
    source = (ROOT / path).read_text(encoding="utf-8")
    matches = re.findall(r'const ' + re.escape(name) + r': &str = r#"(.*?)"#;', source, re.S)
    if len(matches) != 1:
        raise ValueError(f"Expected one shell constant: {name}")
    return matches[0]


def run_acceptance():
    name = os.environ.get("APOLLO_OFFICE_CONTAINER", "")
    if os.environ.get("APOLLO_OFFICE_REAL_DOCKER") != "1" or not re.fullmatch(r"apollo-ci-[0-9]+-[0-9]+", name):
        raise SystemExit("Requires an explicitly authorized disposable CI Docker worker")
    pre = shell_constant("src/sh.rs", "NODE_PRE")
    select = shell_constant("src/pipe_impl.rs", "NODE_VERSION_SELECTION")

    def execute(script):
        return subprocess.run(["docker", "exec", "-i", name, "bash", "-s"],
                              input=script, text=True, check=True, capture_output=True, timeout=90).stdout

    root = execute("set -eu; mktemp -d /work/apollo-f01-node.XXXXXXXX\n").strip()
    if not re.fullmatch(r"/work/apollo-f01-node\.[a-zA-Z0-9]{8}", root):
        raise RuntimeError("Unexpected fixture directory")
    fixture = shlex.quote(root)
    try:
        execute(f"set -eu\ncd {fixture}\nmkdir versioned invalid default\nprintf '20.19.0\\n' > versioned/.node-version\nprintf 'invalid version!\\n' > invalid/.node-version\n")
        # The real lifecycle step has already installed Node 20. Do not install
        # another version or mutate fnm's shared default in this acceptance test.
        execute(pre + f"\nset -eu\ncd {fixture}/versioned\ntest \"$(node --version)\" = v20.19.0\ncd ../default\ntest \"$(node --version)\" = v24.22.3\n")
        execute(pre + f"\nset -eu\ncd {fixture}/invalid\ntest \"$(node --version)\" = v24.22.3\n"
                + f"if timeout 30s bash -c {shlex.quote(select)}; then echo 'Invalid version accepted' >&2; exit 1; else status=$?; test \"$status\" -lt 124; fi\n"
                + "test \"$(node --version)\" = v24.22.3\n")
        setup = 'set -eu; fnm use --silent-if-unchanged 20.19.0; hash -r; test "$(node --version)" = v20.19.0'
        execute(pre + f"\nset -eu\ncd {fixture}/default\nbash -c {shlex.quote(setup)}\n")
        # A new worker shell must not inherit the setup's per-shell override.
        execute(pre + f"\nset -eu\ncd {fixture}/default\ntest \"$(node --version)\" = v24.22.3\n")
    finally:
        execute(f"set -eu\nrm -rf -- {fixture}\n")
    print("F01: Node 20, Node 24, invalid version and isolated setup checks passed")


if __name__ == "__main__":
    run_acceptance()
