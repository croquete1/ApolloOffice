#!/usr/bin/env python3
"""Render the pixel-themed operational dashboard with synthetic local state."""
from __future__ import annotations

import argparse
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


def fixture(active: bool, telemetry: str = "", codex: dict | None = None) -> dict:
    """Build an API-shaped snapshot with a staged Fixer job when requested."""
    stamp = int(time.time() * 1000)
    if telemetry == "skew":
        # Simula um servidor dez minutos atrás do relógio do navegador.
        stamp -= 600_000
    workers = {
        role: {"st": "idle", "say": "", "job": None, "md": "", "hold": False,
               "t": stamp, "mood": "", "cmds": [], "tk": 0}
        for role in ROLES
    }
    jobs, counts, events, prs = [], {}, [], []
    steward_prs = []
    if active:
        workers["fixer"].update({
            "st": "work", "say": "GREEN: implementing the fix for #123",
            "job": 17, "md": "ollama:qwen3:8b", "tk": 1240,
        })
        jobs = [{
            "id": 17, "repo": "example/apollo", "num": 123,
            "kind": "fix", "title": "Validar dashboard",
            "url": "https://github.com/example/apollo/issues/123",
            "st": "fix", "why": "", "pr": None, "t0": stamp, "t1": 0,
            "red_stage": "committed", "red_commit": "synthetic-red-commit",
            "green_commit": None,
        }]
        prs = [{
            "repo": "example/apollo", "num": 44, "st": "open",
            "title": "Fix dashboard issue", "url": "https://github.com/example/apollo/pull/44",
            "upd": stamp, "merged": None,
        }]
        counts = {"fix": 1, "done": 6}
        events = [{"t": stamp, "w": "fixer", "m": "GREEN: implementing the fix for #123", "j": 17}]
        steward_prs = [{
            "repo": "example/apollo", "num": number, "title": f"Steward PR {number}",
            "url": f"https://github.com/example/apollo/pull/{number}",
            "state": "ci_pendente", "priority": number, "next_action": "Aguardar testes",
        } for number in range(1, 41)]
    return {
        "t": stamp, "v": "smoke-fixture", "nr": 1,
        "o": {"off": False, "man": False, "pause": "", "warn": False,
              "net": {"local": True, "gh": True, "an": True}},
        "w": workers, "q": jobs, "dn": [], "pr": prs, "mg": [], "ev": events,
        "n": counts, "done24": 3 if active else 0,
        "pr_steward": {
            "total": 112 if active else 0,
            "blocked": 108 if active else 0,
            "repos": [{"repo": "example/apollo", "total": 112, "blocked": 108}] if active else [],
            "prs": steward_prs,
        },
        "delivery": {"merged_fixes": 1 if active else 0},
        "prod24": [{"g": 2, "n": 1}, {"g": 0, "n": 2}] if active else [],
        "fl": [], "ln": {}, "cil": {},
        "hs": [{"id": "local", "on_": 1, "dk": 1, "ln": 2}],
        "hst": {"local": {"t": stamp, "cpu": 8, "load": 0.7, "mem": [16384, 8192], "disk": [96, 33]}},
        "tdd": {
            "active": True, "repos": ["example/apollo"],
            "primary": "ollama:qwen3:8b", "fallback": "codex:gpt-6-luna",
            "fallback_effort": "max", "reviewer": "cc:opus",
        },
        "lm": None, "star": None,
        # Valores sintéticos exclusivos dos testes, não dados da conta.
        "codex_usage": codex,
    }


class Handler(SimpleHTTPRequestHandler):
    """Serve actual UI files and a controlled API snapshot."""
    state: dict = {}
    quota_mode = "normal"
    result: str | None = None

    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(UI_DIR), **kwargs)

    def do_GET(self):
        if urlsplit(self.path).path in ("/", "/index.html"):
            html = (UI_DIR / "index.html").read_text(encoding="utf-8")
            probe = r"""
<script>
(function () {
  function verify() {
    var errors = [];
    function assert(value, message) { if (!value) errors.push(message); }
    try {
      var app = document.getElementById("aoControlRoom");
      var canvas = document.getElementById("cv");
      var office = document.querySelector(".ao-office");
      var filter = document.getElementById("aoTaskFilter");
      var repoFilter = document.getElementById("aoRepoFilter");
      assert(Boolean(app), "O dashboard operacional não carregou");
      assert(canvas && office && !office.contains(canvas) && canvas.getClientRects().length === 0,
             "O escritório virtual continua visível");
      assert(window.AO.disablePixelArt === true && window.AO.pixelMotion === false,
             "A renderização do escritório não foi desativada");
      if (window.innerWidth <= 600) {
        assert(getComputedStyle(document.querySelector(".ao-metrics")).gridTemplateColumns.split(" ").length === 2,
               "Os indicadores não se reorganizaram em duas colunas móveis");
      }
      assert(document.querySelectorAll("#aoAgentList .ao-agent-row").length === 10,
             "A lista de estado dos dez agentes não carregou");
      assert(Boolean(filter && repoFilter), "Faltam os filtros de tarefa ou projeto");
      assert(Boolean(document.getElementById("aoPipeline")), "O módulo do fluxo de trabalho não carregou");
      assert(Boolean(document.getElementById("aoTasksList") && document.getElementById("aoPRList") &&
                     document.getElementById("aoEventList")), "Falta o módulo de tarefas, PRs ou atividade");
      assert(Boolean(document.getElementById("aoProductivityChart") && document.getElementById("aoMachines")),
             "Falta o módulo de produtividade ou recursos");
      assert(!document.querySelector('link[href="premium.css"]') &&
             !document.querySelector('script[src="office-motion.js"]'),
             "Uma folha ou animação do redesign anterior continua ativa");
      if (window.innerWidth <= 600) {
        assert(document.documentElement.scrollWidth <= window.innerWidth,
               "O dashboard móvel tem overflow horizontal");
      }
      var fixer = document.querySelector('#aoAgentList [data-role="fixer"]');
      if (window.__AO_SMOKE_ACTIVE__) {
        assert(document.getElementById("aoMetricWorkers").textContent.includes("1 / 10"),
               "A contagem de agentes ativos não reflete o Fixer");
        assert(document.getElementById("aoMetricDone").textContent.trim() === "1",
               "O total de correções integradas não reflete o relatório do steward");
        assert(document.getElementById("aoTasksList").textContent.includes("Validar dashboard"),
               "A tarefa real não aparece no módulo de tarefas");
        assert(document.getElementById("aoPRList").textContent.includes("Fix dashboard issue"),
               "O PR real não aparece no módulo de pull requests");
        assert(document.getElementById("aoEventList").textContent.includes("implementing the fix for #123"),
               "O evento real não aparece no módulo de atividade");
        assert(document.getElementById("aoStewardSummary").textContent.includes("112 abertas · 108 por tratar"),
               "A fila do Steward mostra apenas a amostra de PRs em vez do total do relatório");
        assert(document.getElementById("aoRepoFilter").value === "" &&
               Array.from(repoFilter.options).some(option => option.value === "example/apollo"),
               "O filtro não contém o repositório ativo");
        assert(document.querySelectorAll("#aoProductivityChart .ao-merge-graph span").length === 24,
               "A produtividade não mostra as 24 horas");
        assert(document.getElementById("aoMachines").textContent.includes("local"),
               "Os recursos da máquina local não aparecem");
        var staged = document.getElementById("aoStagedPanel");
        var models = document.getElementById("aoStagedModels");
        assert(staged && !staged.hidden, "O módulo TDD test-first não está visível");
        assert(models && models.textContent.includes("Qwen3 8B") &&
               models.textContent.includes("GPT-6 Luna") && models.textContent.includes("Claude Opus"),
               "Os modelos staged não refletem a configuração do controlador");
        assert(document.getElementById("aoStagedStatus").textContent.includes("GREEN em execução"),
               "O estado GREEN não acompanha o trabalho ativo");
        filter.value = "validar dashboard";
        filter.dispatchEvent(new Event("input", {bubbles: true}));
        assert(document.querySelectorAll("#aoTasksList .ao-data-row").length === 1,
               "A pesquisa não encontrou a tarefa correspondente");
        filter.value = "no-matching-task";
        filter.dispatchEvent(new Event("input", {bubbles: true}));
        assert(document.querySelector("#aoTasksList .ao-empty"), "A pesquisa não mostrou o estado sem resultados");
        filter.value = "";
        filter.dispatchEvent(new Event("input", {bubbles: true}));
        repoFilter.value = "example/apollo";
        repoFilter.dispatchEvent(new Event("change", {bubbles: true}));
        assert(document.getElementById("aoTasksList").textContent.includes("Validar dashboard"),
               "O filtro por projeto removeu indevidamente o projeto selecionado");
        repoFilter.value = "";
        repoFilter.dispatchEvent(new Event("change", {bubbles: true}));
        fixer.click();
        var overlay = document.getElementById("aoAgentOverlay");
        assert(overlay && !overlay.hidden, "O detalhe do agente não abriu");
        document.getElementById("aoDetailClose").click();
        assert(overlay.hidden, "O detalhe do agente não fechou");
      } else {
        assert(document.getElementById("aoMetricWorkers").textContent.includes("0 / 10"),
               "Um agente parado foi mostrado como ativo");
        assert(document.getElementById("aoTasksList").textContent.includes("Sem tarefas em aberto"),
               "O estado vazio de tarefas não foi apresentado");
      }
      var buttons = Array.from(document.querySelectorAll(".ao-pixel-tools button"));
      assert(["Custos", "Repositórios", "Máquinas", "Definições"].every(label =>
        buttons.some(button => button.textContent.includes(label))), "Faltam atalhos operacionais do painel base");
    } catch (error) { errors.push(String(error)); }
    var result = errors.length ? "FAIL: " + errors.join(" | ") : "PASS";
    document.documentElement.setAttribute("data-ao-smoke", result);
    var report = new XMLHttpRequest();
    report.open("GET", "/__ao_smoke_result?value=" + encodeURIComponent(result), false);
    report.send(null);
  }
  var started = Date.now();
  (function waitForDashboard() {
    var ready = document.getElementById("aoControlRoom") &&
      document.getElementById("aoMetricWorkers") &&
      document.getElementById("aoAgentList") &&
      document.getElementById("aoMetricWorkers").textContent.trim() !== "—";
    if (ready || Date.now() - started >= 3500) verify();
    else setTimeout(waitForDashboard, 50);
  })();
})();
</script>
"""
            state_json = json.dumps(self.state).replace("<", "\\u003c")
            bootstrap = "<script>window.__AO_SMOKE_ACTIVE__=" + json.dumps(bool(self.state.get("q"))) + ";window.AO.apply(" + state_json + ");</script>"
            html = html.replace("</body>", bootstrap + probe + "</body>")
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
            return self.wfile.write(data)
        if urlsplit(self.path).path == "/__ao_smoke_result":
            Handler.result = parse_qs(urlsplit(self.path).query).get("value", [""])[0]
            self.send_response(204)
            self.end_headers()
            return
        if self.path.startswith("/api/"):
            self.send_error(404, "Synthetic smoke server does not implement this endpoint")
            return
        super().do_GET()

    def log_message(self, *args):
        pass


def choose_browser() -> str:
    for value in (os.environ.get("CHROME_BIN"), "chromium", "chromium-browser",
                  "google-chrome", "google-chrome-stable", "chrome"):
        if value:
            path = shutil.which(value) or (value if Path(value).is_file() else None)
            if path:
                return str(path)
    raise RuntimeError("Chrome/Chromium não está instalado neste ambiente.")


def capture_screenshot(browser: str, url: str, image: Path, viewport: str) -> None:
    image.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="apollo-browser-shot-") as profile:
        command = [browser, "--headless=new", "--no-sandbox", "--disable-gpu",
                   "--disable-dev-shm-usage", "--disable-background-networking",
                   "--no-first-run", "--no-default-browser-check", f"--user-data-dir={profile}",
                   "--virtual-time-budget=1500", f"--window-size={viewport}",
                   f"--screenshot={image}", url]
        result = subprocess.run(command, capture_output=True, text=True, timeout=45)
        if result.returncode != 0 or not image.is_file() or image.stat().st_size == 0:
            raise AssertionError("Captura Chromium falhou: " + result.stderr[-1800:])
    print(f"Screenshot: {image} ({viewport})")


def run_case(browser: str, active: bool, screenshot: Path | None = None,
             viewport: str = "1366,768", query: str = "", modern: bool = True,
             telemetry: str = "", codex: dict | None = None) -> None:
    Handler.state = fixture(active, telemetry, codex)
    Handler.result = None
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    suffix = query or ("" if modern else (
        "?pixel_telemetry=" + telemetry if telemetry else ""
    ))
    url = f"http://127.0.0.1:{server.server_port}/" + suffix
    try:
        with tempfile.TemporaryDirectory(prefix="apollo-browser-") as profile:
            command = [browser, "--headless=new", "--no-sandbox", "--disable-gpu",
                       "--disable-dev-shm-usage", "--disable-background-networking",
                       "--no-first-run", "--no-default-browser-check", "--enable-logging=stderr",
                       f"--user-data-dir={profile}", "--virtual-time-budget=4500",
                       f"--window-size={viewport}", "--dump-dom", url]
            result = subprocess.run(command, capture_output=True, text=True, timeout=45)
            if result.returncode != 0:
                raise AssertionError("Chromium falhou: " + result.stderr[-1800:])
            console_errors = [line for line in result.stderr.splitlines()
                              if "[ERROR:CONSOLE]" in line or "Uncaught " in line]
            if console_errors:
                raise AssertionError("Erros de JavaScript no Chromium: " + " | ".join(console_errors[-8:]))
            dom = result.stdout
            smoke = re.search(r'data-ao-smoke="([^"]+)"', dom)
            if not smoke or smoke.group(1) != "PASS":
                raise AssertionError("Verificação da interface falhou: " +
                                     (smoke.group(1) if smoke else "verificação não executada"))
            for role in ROLES:
                if f'data-role="{role}"' not in dom:
                    raise AssertionError("Posto em falta: " + role)
            if active and "Qwen3 8B" not in dom:
                raise AssertionError("O modelo local Qwen3 8B não aparece no painel TDD.")
            if 'id="aoTaskFilter"' not in dom or 'id="aoRepoFilter"' not in dom:
                raise AssertionError("Os filtros de tarefas e projetos não estão no DOM.")
            label = "Fixer ativo" if active else "agentes em repouso"
            print(f"PASS: Chromium validou módulos pixelizados e estado de {label}.")
        if active and screenshot:
            capture_screenshot(browser, url, screenshot, viewport)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--screenshot", type=Path, help="Optional desktop screenshot path")
    parser.add_argument("--mobile-screenshot", type=Path, help="Optional mobile screenshot path")
    args = parser.parse_args()
    for file in ("index.html", "o.js", "control-room.js", "pixel-dashboard.css"):
        if not (UI_DIR / file).is_file():
            raise RuntimeError("Ficheiro da interface em falta: " + file)
    browser = choose_browser()
    print("Browser:", browser)
    run_case(browser, active=False, viewport="1366,768")
    run_case(browser, active=True, screenshot=args.screenshot, viewport="1656,1146")
    if args.mobile_screenshot:
        run_case(browser, active=True, screenshot=args.mobile_screenshot, viewport="390,844")
    print("PASS: módulos, filtros, pipeline staged, produtividade e recursos.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (AssertionError, RuntimeError, subprocess.TimeoutExpired) as exc:
        print("FAIL:", exc, file=sys.stderr)
        sys.exit(1)
