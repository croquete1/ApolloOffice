/* Painel de limites reais Codex, sem scraping nem leitura de credenciais. */
(() => {
  "use strict";
  const A = window.AO;
  if (!A || typeof A.subscribeState !== "function") return;
  const modern = !!document.getElementById("aoControlRoom");
  const target = modern ? document.querySelector(".ao-bottom-grid") : document.querySelector(".side");
  if (!target || document.getElementById("codexQuota")) return;

  const panel = document.createElement(modern ? "article" : "section");
  panel.id = "codexQuota";
  panel.className = modern ? "ao-panel codex-quota-card" : "card codex-quota-card";
  panel.setAttribute("aria-label", "Limites de utilização do Codex");
  panel.innerHTML = [
    '<div class="codex-quota-head"><h2>Utilização Codex</h2>',
    '<span class="codex-quota-source" id="codexQuotaStatus" role="status">A consultar…</span></div>',
    '<div class="codex-quota-window" data-codex-window="five_hour">',
    '  <div class="codex-quota-line"><strong>Limite de 5 horas</strong><span data-remaining>—</span></div>',
    '  <div class="codex-quota-bar" role="progressbar" aria-label="Quota disponível em 5 horas" aria-valuemin="0" aria-valuemax="100"><i></i></div>',
    '  <div class="codex-quota-reset" data-reset>Reposição: —</div>',
    '</div>',
    '<div class="codex-quota-window" data-codex-window="weekly">',
    '  <div class="codex-quota-line"><strong>Limite semanal</strong><span data-remaining>—</span></div>',
    '  <div class="codex-quota-bar" role="progressbar" aria-label="Quota semanal disponível" aria-valuemin="0" aria-valuemax="100"><i></i></div>',
    '  <div class="codex-quota-reset" data-reset>Reposição: —</div>',
    '</div>',
    '<div class="codex-quota-footer" id="codexQuotaUpdated">A aguardar leitura do Codex CLI</div>'
  ].join("");
  if (!modern) {
    const log = target.querySelector("#log")?.closest(".card");
    if (log) target.insertBefore(panel, log);
    else target.append(panel);
  } else target.append(panel);

  const status = panel.querySelector("#codexQuotaStatus");
  const footer = panel.querySelector("#codexQuotaUpdated");
  const nodes = {
    five_hour: panel.querySelector('[data-codex-window="five_hour"]'),
    weekly: panel.querySelector('[data-codex-window="weekly"]')
  };
  let usage = null;
  let serverTime = NaN;
  let receivedAt = 0;
  // A base temporal é a hora do servidor, atualizada pelo relógio monotónico.
  // Evita contagens incorretas quando os relógios locais divergem.
  const nowMs = () => Number.isFinite(serverTime) && serverTime > 0
    ? serverTime + Math.max(0, performance.now() - receivedAt) : Date.now();
  const formatter = new Intl.DateTimeFormat("pt-PT", {
    day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit"
  });
  const clock = new Intl.DateTimeFormat("pt-PT", {
    hour: "2-digit", minute: "2-digit", second: "2-digit"
  });
  const duration = (seconds) => {
    if (seconds <= 0) return "Reposição prevista; aguarda nova leitura";
    const days = Math.floor(seconds / 86400);
    const hours = Math.floor((seconds % 86400) / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    if (days) return days + " d " + hours + " h " + minutes + " min";
    if (hours) return hours + " h " + minutes + " min";
    return minutes + " min " + (seconds % 60) + " s";
  };
  const renderWindow = (name) => {
    const node = nodes[name], w = usage?.[name];
    const amount = node.querySelector("[data-remaining]");
    const bar = node.querySelector('[role="progressbar"]');
    const fill = bar.querySelector("i");
    const reset = node.querySelector("[data-reset]");
    const remaining = typeof w?.remaining === "number" && Number.isFinite(w.remaining)
      && w.remaining >= 0 && w.remaining <= 100 ? w.remaining : null;
    node.classList.toggle("codex-quota-missing", remaining === null);
    if (remaining === null) {
      amount.textContent = "Indisponível";
      bar.removeAttribute("aria-valuenow");
      fill.style.width = "0%";
      reset.textContent = "Sem dados para esta janela";
      return;
    }
    const pct = Math.round(remaining * 10) / 10;
    amount.textContent = pct.toLocaleString("pt-PT", { maximumFractionDigits: 1 }) + "% restante";
    fill.style.width = pct + "%";
    bar.setAttribute("aria-valuenow", String(pct));
    node.classList.toggle("codex-quota-low", remaining <= 20);
    const ts = Number(w.resets_at);
    if (!Number.isFinite(ts) || ts <= 0) {
      reset.textContent = "Hora de reposição indisponível";
    } else {
      const remainingSec = Math.ceil(ts - nowMs() / 1000);
      reset.textContent = duration(remainingSec) + " · " + formatter.format(new Date(ts * 1000));
    }
  };
  const refresh = () => {
    if (!panel.isConnected) return;
    const at = Number(usage?.updated_at);
    const age = Number.isFinite(at) && at > 0 ? nowMs() - at : Infinity;
    const live = usage?.status === "ok" && age >= -60000 && age < 600000;
    const historical = age < Infinity;
    status.textContent = live ? "Atualizado" : usage?.status === "loading"
      ? "A consultar…" : historical ? "Dados anteriores" : "Sem ligação";
    status.classList.toggle("codex-quota-status-ok", live);
    if (historical) {
      footer.textContent = "Leitura às " + clock.format(new Date(at)) +
        (live ? " · consulta automática a cada 3 min" : " · dados desatualizados");
    } else {
      footer.textContent = usage?.status === "loading"
        ? "A consultar o Codex CLI…" : "Sem dados: confirme a autenticação do Codex CLI";
    }
    renderWindow("five_hour");
    renderWindow("weekly");
  };
  A.subscribeState((s) => {
    serverTime = Number(s?.t);
    receivedAt = performance.now();
    usage = s?.codex_usage || null;
    refresh();
  });
  const timer = setInterval(() => {
    if (!panel.isConnected) { clearInterval(timer); return; }
    refresh();
  }, 1000);
})();
