#!/usr/bin/env python3
"""Headless Chromium smoke test for the real Apollo Office frontend.

Serves only local UI assets and a synthetic /api/state payload. No AI providers,
GitHub write APIs, credentials or live backend are involved.

Run: python3 quality/browser_smoke.py
Requires Chrome/Chromium installed; uses only the Python standard library.
"""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

UI_DIR = Path(__file__).resolve().parent.parent / "ui"
ROLES = ("scout", "guard", "vet", "watcher", "hunter", "repro", "fixer", "ci", "reviewer", "submit")


def fixture(active: bool, telemetry: str = "") -> dict:
    """Construct an API-compatible snapshot, optionally with telemetry anomalies."""
    stamp = int(time.time() * 1000)
    if telemetry == "skew":
        # Simula um servidor dez minutos atrás do relógio do navegador.
        stamp -= 600_000
    workers = {
        role: {"st": "idle", "say": "", "job": None, "md": "", "hold": False,
               "t": stamp, "mood": "", "cmds": [], "tk": 0}
        for role in ROLES
    }
    jobs = []
    counts = {}
    if active:
        workers["fixer"].update({
            "st": "work", "say": "A desenvolver a interface",
            "job": 17, "md": "ollama:qwen3:8b", "tk": 1240,
        })
        jobs = [{
            "id": 17, "repo": "example/apollo", "num": 123,
            "kind": "fix", "title": "Validar dashboard",
            "url": "https://github.com/example/apollo/issues/123",
            "st": "fix", "why": "", "pr": None, "t0": stamp, "t1": 0,
        }]
        counts = {"fix": 1}
    return {
        "t": stamp, "v": "smoke-fixture", "nr": 1,
        "o": {"off": False, "man": False, "pause": "", "warn": False,
              "net": {"local": True, "gh": True, "an": True}},
        "w": workers, "q": jobs, "dn": [], "pr": [],
        "mg": [], "ev": [], "n": counts, "fl": [], "ln": {}, "cil": {},
        "hs": [{"id": "local", "on_": 1}],
        "hst": {"local": {
            "t": stamp - 130_000 if telemetry == "stale" else (
                stamp + 300_000 if telemetry == "future" else stamp
            ),
            "cpu": 8, "load": 1.75,
            "mem": [8192, 16384] if telemetry == "invalid" else [16384, 8192],
            "disk": [40, 60] if telemetry == "invalid" else [100, 40],
            "temp": 45,
        }},
        "lm": None, "star": None,
    }


class Handler(SimpleHTTPRequestHandler):
    """Serve the real files in ui/ and a controlled fixture API."""
    state: dict = {}

    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(UI_DIR), **kwargs)

    def do_GET(self):
        url = urlsplit(self.path)
        query = parse_qs(url.query)
        modern = query.get("ui", [""])[0] == "modern"
        telemetry = query.get("pixel_telemetry", [""])[0]
        if telemetry not in ("fresh", "stale", "skew", "invalid", "future"):
            telemetry = ""
        if url.path in ("/", "/index.html"):
            # Inject a browser-only smoke probe; the repository UI is unchanged.
            html = (UI_DIR / "index.html").read_text(encoding="utf-8")
            probe = """
<script>
(function () {
  function verify() {
    var errors = [];
    function assert(value, message) { if (!value) errors.push(message); }
    try {
      var app = document.getElementById("aoControlRoom");
      var fixer = document.querySelector('.ao-station[data-role="fixer"]');
      var overlay = document.getElementById("aoAgentOverlay");
      assert(Boolean(app), "Control Room indisponível");
      assert(Boolean(fixer), "Personagem Fixer em falta");
      var room = document.querySelector(".ao-office");
      assert(room && room.dataset.visualVersion === "2",
             "O escritório 2.5D premium não está ativo");
      assert(document.querySelectorAll(".ao-v2-character").length === 10,
             "Faltam personagens SVG detalhadas");
      assert(document.querySelectorAll(".ao-v2-window").length === 2,
             "A sala premium não tem as janelas esperadas");
      assert(document.querySelectorAll(".ao-v2-plant").length === 2,
             "Faltam elementos de profundidade da sala");
      if (app && fixer && overlay) {
        fixer.click();
        assert(!overlay.hidden, "O detalhe do agente não abriu");
        document.getElementById("aoDetailClose").click();
        assert(overlay.hidden, "O detalhe do agente não fechou");
        var toggle = document.getElementById("aoMotion");
        toggle.checked = false;
        toggle.dispatchEvent(new Event("change", {bubbles: true}));
        assert(app.classList.contains("ao-motion-disabled"), "Desativar animações falhou");
        assert(getComputedStyle(fixer.querySelector(".ao-v2-character")).animationName === "none",
               "A preferência de movimento desativado não foi aplicada à personagem");
        toggle.checked = true;
        toggle.dispatchEvent(new Event("change", {bubbles: true}));
        assert(!app.classList.contains("ao-motion-disabled"), "Reativar animações falhou");
        var expand = document.getElementById("aoExpand");
        expand.click();
        assert(document.getElementById("aoRoomPanel").classList.contains("ao-expanded"),
               "Ampliar escritório falhou");
        expand.click();
        assert(!document.getElementById("aoRoomPanel").classList.contains("ao-expanded"),
               "Restaurar escritório falhou");
      }
    } catch (error) { errors.push(String(error)); }
    document.documentElement.setAttribute("data-ao-smoke",
      errors.length ? "FAIL: " + errors.join(" | ") : "PASS");
  }
  // Poll until the new UI has a real state snapshot. A fixed sleep can
  // incorrectly fail on slower CI runners. Stop within Chromium's 4500 ms
  // virtual-time budget; verify() will report a useful failure if not ready.
  var started = Date.now();
  (function waitForDashboard() {
    var ready = document.getElementById("aoControlRoom") &&
      document.querySelector('.ao-station[data-role="fixer"]') &&
      document.getElementById("aoMetricWorkers") &&
      document.getElementById("aoMetricWorkers").textContent.includes(" / 10");
    if (ready || Date.now() - started >= 3500) verify();
    else setTimeout(waitForDashboard, 100);
  })();
})();
</script>
"""
            if not modern:
                probe = """
<script>
(function () {
  // O canvas pode existir antes do primeiro frame. Validar que a
  // interface recebeu dados e desenhou píxeis visíveis.
  var started = Date.now();
  function painted(canvas) {
    if (!canvas || canvas.width <= 0 || canvas.height <= 0) return false;
    var ctx = canvas.getContext("2d");
    if (!ctx) return false;
    var w = canvas.width, h = canvas.height;
    var positions = [[0.25, 0.25], [0.5, 0.5], [0.75, 0.75]];
    return positions.some(function (point) {
      var pixel = ctx.getImageData(
        Math.floor(w * point[0]), Math.floor(h * point[1]), 1, 1
      ).data;
      return pixel[3] > 0;
    });
  }
  function inspectTelemetry(canvas, errors) {
    var scenario = "__PIXEL_TELEMETRY__";
    if (!scenario || !canvas) return;
    // Reproduzir o hover real sobre o primeiro host no rack pixelizado.
    var rect = canvas.getBoundingClientRect();
    canvas.dispatchEvent(new MouseEvent("mousemove", {
      bubbles: true,
      clientX: rect.left + rect.width * (272 / 400),
      clientY: rect.top + rect.height * (80 / 258)
    }));
    var tooltip = document.getElementById("tip");
    var text = tooltip && !tooltip.hidden ? tooltip.textContent : "";
    if (!text.includes("este computador"))
      errors.push("Tooltip do host não abriu através do canvas");
    if (scenario === "fresh" || scenario === "skew") {
      if (!text.includes("online") || !text.includes("carga média") ||
          !text.includes("RAM") || !text.includes("disco") ||
          !text.includes("sensor não identificado"))
        errors.push("Telemetria atual não foi apresentada corretamente");
      if (scenario === "skew" && !text.includes("0s atrás"))
        errors.push("Idade da telemetria afetada pelo relógio do navegador");
    } else if (scenario === "stale" || scenario === "future") {
      if (!text.includes("por confirmar") ||
          !text.includes("sem telemetria recente") || text.includes("carga média"))
        errors.push("Telemetria expirada ou futura não foi bloqueada");
    } else if (scenario === "invalid") {
      if (!text.includes("online") || text.includes("RAM") ||
          text.includes("disco") || !text.includes("carga média"))
        errors.push("Valores de RAM/disco fora dos limites foram apresentados");
    }
  }
  function verify() {
    var errors = [];
    var canvas = document.getElementById("cv");
    if (!canvas) errors.push("Canvas pixelizado indisponível");
    if (document.getElementById("aoControlRoom"))
      errors.push("Control Room montado sem opção explícita");
    if (window.AO?.modernRoom === true)
      errors.push("Vista moderna ativada por omissão");
    if (canvas && getComputedStyle(canvas).display === "none")
      errors.push("Canvas pixelizado oculto");
    if (!window.AO?.S) errors.push("Estado real da interface não recebido");
    try {
      if (!painted(canvas)) errors.push("O escritório pixelizado não desenhou nenhum frame");
      inspectTelemetry(canvas, errors);
    } catch (error) {
      errors.push("Falha ao ler o canvas pixelizado: " + String(error));
    }
    document.documentElement.setAttribute("data-ao-smoke",
      errors.length ? "FAIL: " + errors.join(" | ") : "PASS");
  }
  (function waitForFrame() {
    var ready = window.AO?.S && painted(document.getElementById("cv"));
    if (ready || Date.now() - started >= 3500) verify();
    else setTimeout(waitForFrame, 100);
  })();
})();
</script>
"""
                probe = probe.replace("__PIXEL_TELEMETRY__", telemetry)
            html = html.replace("</body>", probe + "</body>")
            payload = html.encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return
        if self.path == "/api/state":
            data = json.dumps(self.state).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        if self.path.startswith("/api/"):
            self.send_error(404, "Mock server does not implement this endpoint")
            return
        super().do_GET()

    def log_message(self, *args):
        pass


def choose_browser() -> str:
    """Locate a system browser without downloading binaries or dependencies."""
    for value in (os.environ.get("CHROME_BIN"), "chromium", "chromium-browser",
                  "google-chrome", "google-chrome-stable", "chrome"):
        if value:
            path = shutil.which(value) or (value if Path(value).is_file() else None)
            if path:
                return str(path)
    raise RuntimeError("Chrome/Chromium não está instalado neste ambiente.")


def run_case(browser: str, active: bool, modern: bool = True,
             telemetry: str = "", query: str = "") -> None:
    """Render the real frontend, selecting a view and optional telemetry fixture."""
    Handler.state = fixture(active, telemetry)
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    suffix = query or ("?ui=modern" if modern else (
        "?pixel_telemetry=" + telemetry if telemetry else ""
    ))
    url = f"http://127.0.0.1:{server.server_port}/" + suffix
    try:
        with tempfile.TemporaryDirectory(prefix="apollo-browser-") as profile:
            command = [
                browser, "--headless=new", "--no-sandbox", "--disable-gpu",
                "--disable-dev-shm-usage", "--disable-background-networking",
                "--no-first-run", "--no-default-browser-check",
                f"--user-data-dir={profile}", "--virtual-time-budget=4500",
                "--dump-dom", url,
            ]
            result = subprocess.run(command, capture_output=True, text=True, timeout=45)
            if result.returncode != 0:
                raise AssertionError("Chromium falhou: " + result.stderr[-1800:])
            dom = result.stdout
            smoke = re.search(r'data-ao-smoke="([^"]+)"', dom)
            if not smoke or smoke.group(1) != "PASS":
                raise AssertionError("Interações do navegador falharam: " +
                                     (smoke.group(1) if smoke else "teste não executado"))
            if modern:
                if 'id="aoControlRoom"' not in dom:
                    raise AssertionError("O Control Room não foi montado. " + dom[-1200:])
                for role in ROLES:
                    if f'data-role="{role}"' not in dom:
                        raise AssertionError("Posto em falta: " + role)
                if len(re.findall(r'class="ao-pipeline-stage\b', dom)) != 10:
                    raise AssertionError("O pipeline não apresenta as dez fases reais.")
                expected = "1 / 10" if active else "0 / 10"
                if not re.search(r'id="aoMetricWorkers"[^>]*>\s*' + re.escape(expected), dom):
                    raise AssertionError("Contagem incorreta de agentes ativos: " + expected)
                if active and "qwen3:8b" not in dom:
                    raise AssertionError("O modelo local Qwen 3 8B não aparece no dashboard.")
                if "Ações Rápidas" in dom:
                    raise AssertionError("A interface ainda apresenta ações rápidas.")
                label = "trabalho real" if active else "repouso"
                print(f"PASS: Chrome/Chromium validou renderização e interações de {label}.")
            else:
                if 'id="aoControlRoom"' in dom:
                    raise AssertionError("Vista moderna ativada sem autorização.")
                if 'id="cv"' not in dom:
                    raise AssertionError("Canvas pixelizado não encontrado.")
                print("PASS: escritório pixelizado" +
                      (f" · telemetria {telemetry}" if telemetry else
                       (f" · consulta {query}" if query else " predefinido")) + ".")
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def main() -> int:
    """Run the original pixel office and optional modern views in a real browser."""
    if not (UI_DIR / "control-room.js").is_file():
        raise RuntimeError("Os ficheiros reais da interface estão em falta.")
    browser = choose_browser()
    print("Browser:", browser)
    run_case(browser, active=False, modern=False)
    run_case(browser, active=False, modern=True)
    run_case(browser, active=True, modern=True)
    for scenario in ("fresh", "stale", "skew", "invalid", "future"):
        run_case(browser, active=False, modern=False, telemetry=scenario)
    # As semelhanças textuais não podem ativar a vista moderna.
    run_case(browser, active=False, modern=False, query="?ui=modern2")
    run_case(browser, active=False, modern=False, query="?x=ui%3Dmodern")
    print("PASS: 10 cenários de navegador: pixel/modern, telemetria e query exata.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (AssertionError, RuntimeError, subprocess.TimeoutExpired) as exc:
        print("FAIL:", exc, file=sys.stderr)
        sys.exit(1)
