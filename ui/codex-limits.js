/* Read-only Codex quota panel. Values always originate from the local CLI. */
(() => {
  "use strict";
  const root = document.getElementById("codexLimits");
  if (!root) return;
  // In optional modern mode the legacy header is hidden; move the same panel.
  if (window.AO?.modernRoom) {
    document.querySelector(".ao-top-actions")?.prepend(root);
  }

  const $ = (id) => document.getElementById(id);
  const windows = [
    { key: "fiveHour", pct: $("codexFivePct"), reset: $("codexFiveReset"), fill: $("codexFiveFill"), meter: $("codexFiveMeter"), row: $("codexFiveRow") },
    { key: "weekly", pct: $("codexWeekPct"), reset: $("codexWeekReset"), fill: $("codexWeekFill"), meter: $("codexWeekMeter"), row: $("codexWeekRow") }
  ];
  const stamp = $("codexUpdated");
  const REFRESH_MS = 120_000;
  let limits = null;
  let serverTime = null;
  let fetchedAt = 0;
  let lastFetch = 0;
  let updating = false;

  const clamp = (n) => Math.max(0, Math.min(100, n));
  const until = (seconds) => {
    if (!Number.isFinite(seconds) || seconds <= 0) return "A aguardar reposição";
    const mins = Math.ceil(seconds / 60);
    const days = Math.floor(mins / 1440);
    const hours = Math.floor((mins % 1440) / 60);
    const minutes = mins % 60;
    if (days) return days + " d " + hours + " h";
    if (hours) return hours + " h " + minutes + " min";
    return mins + " min";
  };
  const timeNow = () => serverTime != null
    ? serverTime + Math.max(0, performance.now() - fetchedAt)
    : Date.now();

  const showWindow = (w) => {
    const data = limits?.[w.key];
    const ok = data && Number.isFinite(data.remainingPercent) && Number.isFinite(data.resetsAt);
    if (!ok) {
      w.pct.textContent = "—";
      w.reset.textContent = "Sem dados";
      w.reset.title = "";
      w.fill.style.width = "0%";
      w.fill.classList.remove("low");
      w.meter.removeAttribute("aria-valuenow");
      w.meter.setAttribute("aria-valuetext", "Não disponível");
      return;
    }
    const remaining = clamp(data.remainingPercent);
    const label = Math.round(remaining) + "%";
    w.pct.textContent = label;
    w.pct.title = "Quota restante: " + label;
    w.fill.style.width = remaining + "%";
    w.fill.classList.toggle("low", remaining <= 20);
    w.meter.setAttribute("aria-valuenow", String(Math.round(remaining)));
    w.meter.setAttribute("aria-valuetext", label + " restante");
    w.reset.textContent = until(data.resetsAt - timeNow() / 1000);
    w.reset.title = "Reposição prevista: " + new Date(data.resetsAt * 1000).toLocaleString("pt-PT");
  };

  const paint = () => windows.forEach(showWindow);

  const refresh = async () => {
    if (updating) return;
    updating = true;
    lastFetch = Date.now();
    try {
      const response = await fetch("/api/codex/limits", { credentials: "same-origin", cache: "no-store" });
      if (!response.ok) throw new Error("Falha ao consultar o servidor");
      const data = await response.json();
      if (data.status !== "ok") {
        limits = null;
        stamp.textContent = "Indisponível";
        stamp.title = data.message || "Inicie sessão no Codex CLI neste computador";
      } else {
        limits = data;
        serverTime = Number.isFinite(data.serverTime) ? data.serverTime : Date.now();
        fetchedAt = performance.now();
        stamp.textContent = "Atualizado";
        stamp.title = data.checkedAt
          ? "Última consulta: " + new Date(data.checkedAt).toLocaleString("pt-PT")
          : "Limites fornecidos pelo Codex CLI";
      }
    } catch {
      limits = null; // Nunca apresentar números antigos como atuais.
      stamp.textContent = "Sem ligação";
      stamp.title = "Não foi possível consultar os limites. Nova tentativa automática.";
    } finally {
      updating = false;
      paint();
    }
  };

  paint();
  void refresh();
  setInterval(paint, 1000); // Contagem decrescente local; não interroga o Codex.
  setInterval(() => {
    if (!document.hidden) void refresh();
  }, REFRESH_MS);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && Date.now() - lastFetch >= REFRESH_MS) void refresh();
  });
})();
