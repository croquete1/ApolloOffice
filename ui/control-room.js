/* Apollo Office Control Room
 * Presentation-only enhancement. Reads AO.S; never fabricates work or modifies jobs.
 */
(() => {
  "use strict";
  const A = window.AO;
  if (!A || document.querySelector(".ao-app")) return;
  const byId = (id) => document.getElementById(id);
  const escapeHTML = (value) => A.esc(value ?? "");
  const names = A.NM || {};
  const roles = ["scout", "guard", "vet", "watcher", "hunter", "repro", "fixer", "ci", "reviewer", "submit"];
  const colors = ["#3ac9b6", "#5e91ef", "#a283e8", "#4dbee4", "#d1aa6c", "#ef7b6b", "#f39963", "#8792a8", "#d77cb3", "#69c97c"];
  const skins = ["#eab58c", "#bd896c", "#dcac8b", "#d49c77", "#e7b58f", "#b98263", "#f1c5a3", "#d7a082", "#e4b391", "#d4a17f"];
  const hairColors = ["#58372c", "#242e40", "#342c2a", "#a45635", "#58372b", "#303039", "#4f302b", "#2c3546", "#403045", "#59402d"];
  const workerStages = {
    hunter: ["hunt"], repro: ["repro"], fixer: ["fix"], ci: ["ci"],
    reviewer: ["review"], submit: ["gate", "submit"], vet: ["vet"]
  };
  const stageLabels = {
    ready: "Pendente", vet: "Triagem", hunt: "Pesquisa", repro: "Reprodução",
    fix: "Correção", ci: "Testes", review: "Revisão", gate: "Validação",
    approve: "Aprovação", submit: "Submissão", done: "Concluída", drop: "Descartada"
  };
  const formatNum = (n) => Number(n || 0).toLocaleString("pt-PT");
  const safeArray = (v) => Array.isArray(v) ? v : [];
  const datum = (v) => {
    const d = v ? new Date(v) : null;
    return d && !Number.isNaN(d.getTime()) ? d.toLocaleTimeString("pt-PT", {hour:"2-digit",minute:"2-digit"}) : "—";
  };
  const html = [
    '<div class="ao-app" id="aoControlRoom">',
    '  <aside class="ao-sidebar">',
    '    <div class="ao-brand"><span class="ao-logo" aria-hidden="true">A</span><span><strong>Apollo Office</strong><small>Control Center</small></span></div>',
    '    <nav class="ao-navigation" aria-label="Navegação principal">',
    '      <button class="ao-nav-item is-active" type="button" data-target="aoOverview">Painel</button>',
    '      <button class="ao-nav-item" type="button" data-target="aoAgents">Agentes</button>',
    '      <button class="ao-nav-item" type="button" data-target="aoTasks">Tarefas</button>',
    '      <button class="ao-nav-item" type="button" data-target="aoPulls">Pull Requests</button>',
    '      <button class="ao-nav-item" type="button" data-open="repos">Repositórios</button>',
    '      <button class="ao-nav-item" type="button" data-open="hosts">Máquinas</button>',
    '      <button class="ao-nav-item" type="button" data-open="costs">Custos</button>',
    '      <button class="ao-nav-item" type="button" data-target="aoActivity">Atividade</button>',
    '      <button class="ao-nav-item" type="button" data-open="settings">Definições</button>',
    '    </nav>',
    '    <div class="ao-sidebar-section"><span>Repositórios em atividade</span><div id="aoRepos" class="ao-repo-list"><div class="ao-muted">A carregar…</div></div></div>',
    '    <div class="ao-sidebar-footer"><span class="ao-presence-dot"></span><span id="aoConnection">A estabelecer ligação…</span></div>',
    '  </aside>',
    '  <div class="ao-workspace">',
    '    <header class="ao-topbar"><div class="ao-top-heading"><strong>Visão geral</strong><span>Operações e agentes em tempo real</span></div>',
    '      <div class="ao-top-actions"><span id="aoOfficeStatus" class="ao-status-chip" role="status" aria-live="polite">A ligar…</span><button id="aoPause" type="button" class="ao-outline-button" disabled>Pausar escritório</button></div>',
    '    </header>',
    '    <main class="ao-content" id="aoOverview">',
    '      <section class="ao-metrics" aria-label="Indicadores operacionais">',
    '        <div class="ao-metric"><span>Agentes a trabalhar</span><strong id="aoMetricWorkers">—</strong><small>Estados reais</small></div>',
    '        <div class="ao-metric"><span>Tarefas em aberto</span><strong id="aoMetricQueue">—</strong><small>Total registado no sistema</small></div>',
    '        <div class="ao-metric"><span>PRs abertos (recentes)</span><strong id="aoMetricPR">—</strong><small>Até 40 PRs registados</small></div>',
    '        <div class="ao-metric"><span>Tarefas concluídas</span><strong id="aoMetricDone">—</strong><small>Total registado no sistema</small></div>',
    '      </section>',
    '      <section class="ao-panel ao-pipeline-panel" aria-label="Fluxo de trabalho">',
    '        <div class="ao-panel-heading"><div><h2>Fluxo de trabalho</h2><p>Contagem total de tarefas por fase, diretamente da base de dados</p></div></div>',
    '        <div id="aoPipeline" class="ao-pipeline"></div>',
    '      </section>',
    '      <section class="ao-primary-grid" aria-label="Escritório e agentes">',
    '        <article class="ao-panel ao-room-panel" id="aoRoomPanel">',
    '          <div class="ao-panel-heading"><div><h2>Escritório virtual</h2><p>Personagens animadas · atividade real</p></div>',
    '            <div class="ao-room-controls"><label class="ao-motion-label" for="aoMotion">Animações</label><input id="aoMotion" type="checkbox" checked aria-label="Ativar animações"><button id="aoExpand" type="button" class="ao-icon-button" aria-label="Ampliar o escritório" aria-controls="aoRoomPanel" aria-expanded="false" title="Ampliar o escritório">⤢</button></div>',
    '          </div>',
    '          <div class="ao-office" role="group" aria-label="Dez agentes no escritório virtual">',
    '            <div class="ao-room-glow" aria-hidden="true"></div>',
    '            <div class="ao-windows" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i></div>',
    '            <div class="ao-office-sign" aria-hidden="true"><b>APOLLO</b><span>DEVELOPMENT OFFICE</span></div>',
    '            <div class="ao-office-lamp ao-lamp-one" aria-hidden="true"></div><div class="ao-office-lamp ao-lamp-two" aria-hidden="true"></div>',
    '            <div class="ao-office-floor" aria-hidden="true"></div>',
    '            <div class="ao-stations" id="aoStations"></div>',
    '          </div>',
    '        </article>',
    '        <article class="ao-panel ao-agent-panel" id="aoAgents">',
    '          <div class="ao-panel-heading"><div><h2>Estado dos agentes</h2><p id="aoAgentSummary">Sem informação</p></div></div>',
    '          <div id="aoAgentList" class="ao-agent-list"></div>',
    '        </article>',
    '      </section>',
    '      <section class="ao-operations-grid" aria-label="Operações">',
    '        <article class="ao-panel" id="aoTasks"><div class="ao-panel-heading"><div><h2>Tarefas em aberto</h2><p>Fluxo de trabalho</p></div></div><div id="aoTasksList" class="ao-list"></div></article>',
    '        <article class="ao-panel" id="aoPulls"><div class="ao-panel-heading"><div><h2>Pull Requests</h2><p>Revisão e integração</p></div></div><div id="aoPRList" class="ao-list"></div></article>',
    '        <article class="ao-panel" id="aoActivity"><div class="ao-panel-heading"><div><h2>Atividade recente</h2><p>Eventos do sistema</p></div></div><div id="aoEventList" class="ao-list"></div></article>',
    '      </section>',
    '      <section class="ao-bottom-grid" aria-label="Métricas adicionais">',
    '        <article class="ao-panel"><div class="ao-panel-heading"><div><h2>Integrações (30 dias)</h2><p>PRs integrados por dia</p></div></div><div class="ao-trend" id="aoMergeChart"></div></article>',
    '        <article class="ao-panel"><div class="ao-panel-heading"><div><h2>Máquinas e recursos</h2><p>Dados disponíveis no Apollo Office</p></div></div><div id="aoMachines" class="ao-machines"></div></article>',
    '      </section>',
    '    </main>',
    '  </div>',
    '  <div class="ao-agent-overlay" id="aoAgentOverlay" hidden>',
    '    <section class="ao-agent-dialog" role="dialog" aria-modal="true" aria-labelledby="aoDetailTitle" tabindex="-1">',
    '      <button class="ao-dialog-close" id="aoDetailClose" type="button" aria-label="Fechar detalhes">×</button>',
    '      <h2 id="aoDetailTitle"></h2><div id="aoDetailBody"></div>',
    '      <div class="ao-detail-actions"><button id="aoDetailJob" type="button" hidden>Ver tarefa</button><button id="aoDetailModel" type="button" hidden>Configurar modelo</button></div>',
    '    </section>',
    '  </div>',
    '</div>'
  ].join("\n");

  const legacy = document.createElement("div");
  legacy.className = "ao-legacy";
  legacy.setAttribute("aria-hidden", "true");
  const drawer = byId("dr");
  for (const node of Array.from(document.body.children)) {
    if (node.tagName === "SCRIPT" || node === drawer || node.tagName === "LINK") continue;
    legacy.appendChild(node);
  }
  document.body.prepend(legacy);
  const holder = document.createElement("div");
  holder.innerHTML = html;
  const app = holder.firstElementChild;
  document.body.appendChild(app);
  if (drawer) document.body.appendChild(drawer);
  document.body.classList.add("ao-enabled");
  A.modernRoom = true;

  const hairs = [
    "M27 45Q24 22 50 18Q68 13 78 32Q86 42 77 49L69 31L61 40L53 28L43 40L35 35Z",
    "M26 48Q22 17 50 16Q80 15 83 40L78 50L71 29Q50 17 36 39Z",
    "M24 43Q23 21 45 22Q69 12 83 34L82 50L29 49Z",
    "M24 49Q19 26 40 18Q66 9 79 25Q87 39 81 57L72 39L37 39Z",
    "M24 45Q19 16 48 18Q67 11 83 34L79 43L69 28L50 31L31 47Z",
    "M23 48Q20 24 42 19Q65 11 81 29L81 48Q66 23 33 45Z",
    "M24 44Q27 19 51 19Q80 19 82 45L71 28L59 35L44 30L31 47Z",
    "M24 46Q27 21 46 19Q70 14 80 38L81 51L67 31L47 30L29 47Z",
    "M23 46Q23 25 41 18Q68 9 81 32L84 54L75 48L72 34L30 48Z",
    "M24 47Q20 23 47 18Q73 17 83 41L80 49L68 31L38 36Z"
  ];
  const character = (i) => {
    const skin = skins[i], color = colors[i], hair = hairColors[i], id = "ao-face-" + i;
    return '<svg class="ao-person" viewBox="0 0 108 125" aria-hidden="true" focusable="false">' +
      '<defs><linearGradient id="' + id + '" x1="0" y1="0" x2="1" y2="1"><stop stop-color="' + skin + '"/><stop offset="1" stop-color="#966751"/></linearGradient></defs>' +
      '<ellipse cx="54" cy="120" rx="30" ry="4" fill="#02060f" opacity=".3"/>' +
      '<path d="M35 87L28 113H44L52 99L64 113H80L71 87Z" fill="#192434"/>' +
      '<path d="M25 77Q33 65 48 66H64Q79 67 86 80L82 105H27Z" fill="' + color + '"/>' +
      '<path d="M46 67L55 86L64 67" fill="#ecf0f5" opacity=".65"/>' +
      '<path d="M32 80L29 107M77 80L81 107" stroke="#152435" stroke-width="3" opacity=".5"/>' +
      '<g class="ao-arm ao-arm-left"><path d="M31 81Q18 86 22 103L38 105" fill="none" stroke="' + color + '" stroke-width="14" stroke-linecap="round"/><ellipse cx="39" cy="105" rx="7" ry="5" fill="' + skin + '"/></g>' +
      '<g class="ao-arm ao-arm-right"><path d="M77 81Q91 87 86 103L70 105" fill="none" stroke="' + color + '" stroke-width="14" stroke-linecap="round"/><ellipse cx="69" cy="105" rx="7" ry="5" fill="' + skin + '"/></g>' +
      '<path d="M45 61V70Q54 79 64 69V61Z" fill="' + skin + '"/>' +
      '<g class="ao-head"><ellipse cx="54" cy="42" rx="28" ry="30" fill="url(#' + id + ')"/>' +
      '<path d="' + hairs[i] + '" fill="' + hair + '"/>' +
      '<path d="M28 48Q22 47 26 55L30 59M79 48Q85 47 82 55L78 59" stroke="' + skin + '" stroke-width="6" stroke-linecap="round"/>' +
      '<g class="ao-eyes"><ellipse cx="44" cy="46" rx="2.3" ry="3.3" fill="#172330"/><ellipse cx="65" cy="46" rx="2.3" ry="3.3" fill="#172330"/></g>' +
      '<path d="M51 55Q55 58 59 55" stroke="#945d52" fill="none" stroke-width="1.8" stroke-linecap="round"/>' +
      (i % 3 === 1 ? '<path d="M24 41Q20 28 30 25M83 41Q87 27 77 25" stroke="#233449" stroke-width="5" fill="none" stroke-linecap="round"/><rect x="21" y="40" width="7" height="18" rx="4" fill="#37495d"/><rect x="79" y="40" width="7" height="18" rx="4" fill="#37495d"/>' : '') +
      (i === 2 || i === 9 ? '<path d="M27 31Q54 8 82 32" stroke="' + hair + '" stroke-width="7" fill="none"/>' : '') +
      '</g></svg>';
  };
  const stations = byId("aoStations");
  stations.innerHTML = roles.map((role, i) => {
    return '<button class="ao-station" type="button" data-role="' + role + '" data-mode="unknown" style="--agent-accent:' + colors[i] + '" aria-label="Ver agente ' + names[role] + ' · sem dados">' +
      '<span class="ao-monitor"><span class="ao-screen"><i></i><i></i><i></i><i></i></span><span class="ao-result-mark" aria-hidden="true"></span></span>' +
      '<span class="ao-chair"></span><span class="ao-avatar-holder">' + character(i) + '</span>' +
      '<span class="ao-desk"><span class="ao-keyboard"></span></span>' +
      '<span class="ao-station-name">' + names[role] + '</span><span class="ao-station-state">A ligar…</span>' +
      '</button>';
  }).join("");
  const stationNodes = Object.fromEntries(roles.map((role) => [role, stations.querySelector('[data-role="' + role + '"]')]));
  let snapshot = null, selected = null, selectedRepo = "", lastFocus = null;
  let lastRepoSignature = "", lastListSignature = "", lastMergeSignature = "", lastPipelineSignature = "";
  let lastMachineSignature = "", lastDetailSignature = "", lastStateAt = 0;
  let animationsOn = true;
  try { animationsOn = localStorage.getItem("ao.motion") !== "off"; } catch {}
  const motion = byId("aoMotion");
  motion.checked = animationsOn;
  const syncMotion = () => app.classList.toggle("ao-motion-disabled", !animationsOn);
  syncMotion();
  motion.addEventListener("change", () => {
    animationsOn = motion.checked;
    try { localStorage.setItem("ao.motion", animationsOn ? "on" : "off"); } catch {}
    syncMotion();
  });
  byId("aoExpand").addEventListener("click", (e) => {
    const expanded = byId("aoRoomPanel").classList.toggle("ao-expanded");
    e.currentTarget.setAttribute("aria-label", expanded ? "Reduzir o escritório" : "Ampliar o escritório");
    e.currentTarget.setAttribute("aria-expanded", String(expanded));
    e.currentTarget.textContent = expanded ? "↙" : "⤢";
  });
  const offline = (s) => Boolean(s?.o?.off);
  const paused = (s) => Boolean(s?.o?.man || s?.o?.pause || s?.o?.off);
  const work = (s, role) => {
    const worker = s?.w?.[role] || {};
    return worker.st === "work" ||
      safeArray(s?.q).some((j) => workerStages[role]?.includes(j.st));
  };
  const mode = (s, role) => {
    const w = s?.w?.[role] || {};
    if (w.hold || paused(s)) return "paused";
    if (work(s, role)) return "work";
    return w.st === "sleep" ? "sleep" : "idle";
  };
  const statusText = {work:"A trabalhar",idle:"Disponível",sleep:"Em repouso",paused:"Em pausa"};
  const resultText = {ok:"Último resultado: OK",sad:"Último resultado: problema",err:"Último resultado: erro"};
  const lastCommands = (worker) => safeArray(worker.cmds).filter((entry) =>
    Array.isArray(entry) && entry.length >= 2 && typeof entry[1] === "string").slice(-4).reverse();
  const updateAgents = (s) => {
    let working = 0;
    const list = byId("aoAgentList");
    if (!list.children.length) {
      list.innerHTML = roles.map((role, i) => '<button type="button" class="ao-agent-row" data-agent="' + role + '"><span class="ao-agent-color" style="background:' + colors[i] + '"></span><span class="ao-agent-title">' + escapeHTML(names[role]) + '</span><span class="ao-agent-status">Sem dados</span></button>').join("");
    }
    for (let i=0;i<roles.length;i++) {
      const role = roles[i], w = s.w?.[role] || {}, el = stationNodes[role], state = mode(s, role);
      if (state === "work") working++;
      if (el.dataset.mode === "work" && state !== "work" && !paused(s) && animationsOn &&
          safeArray(s.dn).some((j) => j.st === "done" && String(j.id) === el.dataset.lastJob)) {
        el.classList.remove("ao-celebrate");
        void el.offsetWidth;
        el.classList.add("ao-celebrate");
      }
      if (w.job) el.dataset.lastJob = String(w.job);
      el.dataset.mode = state;
      // A subtle screen reaction only when a genuinely new command is reported.
      const latestCommand = lastCommands(w)[0];
      if (latestCommand) {
        const commandKey = String(latestCommand[0]) + ":" + latestCommand[1];
        if (el.dataset.lastCommand && el.dataset.lastCommand !== commandKey &&
            state === "work" && animationsOn && !document.hidden &&
            !window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches) {
          el.classList.remove("ao-action-pulse");
          void el.offsetWidth;
          el.classList.add("ao-action-pulse");
        }
        el.dataset.lastCommand = commandKey;
      }
      const result = Object.hasOwn(resultText, w.mood) ? w.mood : "";
      el.dataset.mood = result;
      el.querySelector(".ao-result-mark").textContent = result === "ok" ? "✓" : result === "err" ? "!" : result === "sad" ? "!" : "";
      el.querySelector(".ao-station-state").textContent = statusText[state];
      el.title = [names[role], statusText[state], resultText[result], w.say].filter(Boolean).join(" · ");
      const accessible = ["Ver agente " + names[role], statusText[state], resultText[result]].filter(Boolean).join(". ");
      if (el.getAttribute("aria-label") !== accessible) el.setAttribute("aria-label", accessible);
      const status = list.querySelector('[data-agent="' + role + '"] .ao-agent-status');
      status.className = "ao-agent-status ao-state-" + state;
      status.textContent = statusText[state];
    }
    byId("aoMetricWorkers").textContent = working + " / " + roles.length;
    byId("aoAgentSummary").textContent = working + " em execução · " + (roles.length - working) + " restantes";
  };
  const rowHTML = (label, caption, badge, attr, val, extraClass = "") => {
    const unavailable = attr === "data-pr-url" && !val;
    return '<button class="ao-data-row ' + extraClass + '" type="button" ' +
      (unavailable ? 'disabled title="Ligação indisponível"' : attr + '="' + escapeHTML(val) + '"') + '>' +
      '<span class="ao-row-main"><strong>' + escapeHTML(label) + '</strong><small>' + escapeHTML(caption) + '</small></span>' +
      '<span class="ao-row-badge">' + escapeHTML(badge) + '</span></button>';
  };
  const filtered = (items) => safeArray(items).filter((x) => !selectedRepo || x.repo === selectedRepo);
  const renderLists = (s) => {
    const q = filtered(s.q), prs = filtered(s.pr);
    const totals = countTotals(s);
    byId("aoMetricQueue").textContent = formatNum(totals.active);
    byId("aoMetricPR").textContent = formatNum(safeArray(s.pr).filter((x) => x.st === "open").length);
    byId("aoMetricDone").textContent = formatNum(totals.done);
    const signature = JSON.stringify([selectedRepo, q, prs, safeArray(s.ev).slice(0, 8)]);
    if (signature === lastListSignature) return;
    lastListSignature = signature;
    byId("aoTasksList").innerHTML = q.length ? q.slice(0, 7).map((j) =>
      rowHTML(j.title, j.repo + (j.num ? " · #" + j.num : ""), stageLabels[j.st] || j.st, "data-job", j.id)).join("") :
      '<p class="ao-empty">Sem tarefas em aberto.</p>';
    byId("aoPRList").innerHTML = prs.length ? prs.slice(0, 7).map((p) =>
      rowHTML(p.title, p.repo + " · #" + p.num, p.st === "open" ? "Aberto" : p.st === "merged" ? "Integrado" : "Fechado", "data-pr-url", p.url || "")).join("") :
      '<p class="ao-empty">Sem pull requests registados.</p>';
    byId("aoEventList").innerHTML = safeArray(s.ev).slice(0, 8).map((e) =>
      '<div class="ao-event"><time>' + datum(e.t) + '</time><span class="ao-event-agent">' + escapeHTML(names[e.w] || e.w) +
      '</span><span class="ao-event-message" title="' + escapeHTML(e.m) + '">' + escapeHTML(e.m) + '</span></div>').join("") ||
      '<p class="ao-empty">Sem atividade recente.</p>';
  };
  const renderRepos = (s) => {
    const repos = [...new Set(safeArray(s.q).concat(safeArray(s.dn), safeArray(s.pr)).map((v) => v.repo).filter(Boolean))].sort();
    if (selectedRepo && !repos.includes(selectedRepo)) selectedRepo = "";
    const signature = JSON.stringify([repos, selectedRepo]);
    if (signature === lastRepoSignature) return;
    lastRepoSignature = signature;
    const wrap = byId("aoRepos");
    wrap.replaceChildren();
    const all = document.createElement("button");
    all.type = "button"; all.className = "ao-repo-item" + (!selectedRepo ? " is-selected" : "");
    all.textContent = "Todos";
    all.addEventListener("click", () => { selectedRepo = ""; renderRepos(snapshot); renderLists(snapshot); });
    wrap.appendChild(all);
    for (const repo of repos) {
      const item = document.createElement("button");
      item.type = "button"; item.className = "ao-repo-item" + (selectedRepo === repo ? " is-selected" : "");
      item.textContent = repo;
      item.title = repo;
      item.addEventListener("click", () => { selectedRepo = repo; renderRepos(snapshot); renderLists(snapshot); });
      wrap.appendChild(item);
    }
    if (!repos.length) {
      const empty = document.createElement("div");
      empty.className = "ao-muted"; empty.textContent = "Sem repositórios nos registos atuais.";
      wrap.appendChild(empty);
    }
  };
  // Unlike the limited queue preview, s.n contains the server's full database counts.
  const pipelineSteps = [
    {label: "Triagem", statuses: ["vet", "ready"]},
    {label: "Pesquisa", statuses: ["hunt"]},
    {label: "Reprodução", statuses: ["repro"]},
    {label: "Correção", statuses: ["fix"]},
    {label: "Testes", statuses: ["ci"]},
    {label: "Revisão", statuses: ["review"]},
    {label: "Aprovação", statuses: ["gate", "approve", "submit"]}
  ];
  const pipeline = byId("aoPipeline");
  pipeline.innerHTML = pipelineSteps.map(({label}, i) =>
    '<div class="ao-pipeline-stage" data-pipeline-index="' + i +
    '"><span class="ao-pipeline-index">' + String(i + 1).padStart(2, "0") +
    '</span><strong>' + label +
    '</strong><span class="ao-pipeline-value">—</span></div>'
  ).join("");
  const renderPipeline = (s) => {
    const counts = s.n && typeof s.n === "object" ? s.n : {};
    const values = pipelineSteps.map(({statuses}) =>
      statuses.reduce((sum, key) => sum + Math.max(0, Number(counts[key]) || 0), 0)
    );
    const signature = JSON.stringify(values);
    if (signature === lastPipelineSignature) return;
    lastPipelineSignature = signature;
    for (let i = 0; i < values.length; i++) {
      const item = pipeline.children[i];
      item.classList.toggle("is-busy", values[i] > 0);
      item.querySelector(".ao-pipeline-value").textContent = formatNum(values[i]);
      item.setAttribute("aria-label", pipelineSteps[i].label + ": " + formatNum(values[i]) + " tarefas");
    }
  };
  const countTotals = (s) => {
    const counts = s.n && typeof s.n === "object" ? s.n : {};
    const active = Object.entries(counts)
      .filter(([key]) => key !== "done" && key !== "drop")
      .reduce((total, [, amount]) => total + Math.max(0, Number(amount) || 0), 0);
    return {active, done: Math.max(0, Number(counts.done) || 0)};
  };
  const renderMerges = (s) => {
    const signature = JSON.stringify([s.mg, new Date().toDateString()]);
    if (signature === lastMergeSignature) return;
    lastMergeSignature = signature;
    const counts = new Map(safeArray(s.mg).map((item) => [item.d, Number(item.n) || 0]));
    const days = [];
    for (let i=29;i>=0;i--) {
      const date = new Date(); date.setHours(12,0,0,0); date.setDate(date.getDate()-i);
      const key = [date.getFullYear(), String(date.getMonth()+1).padStart(2,"0"), String(date.getDate()).padStart(2,"0")].join("-");
      days.push({date:key,count:counts.get(key)||0});
    }
    const max = Math.max(1,...days.map((d)=>d.count));
    byId("aoMergeChart").innerHTML = '<div class="ao-merge-graph" role="img" aria-label="Gráfico de integrações dos últimos 30 dias">' +
      days.map((d) => '<span title="' + d.date + ': ' + d.count + ' PRs" style="height:' + Math.max(2,d.count/max*100) + '%" class="' + (d.count?"ao-bar-live":"ao-bar-zero") + '"></span>').join("") +
      '</div><div class="ao-trend-labels"><span>Há 30 dias</span><span>Hoje</span></div>';
  };
  const renderMachines = (s) => {
    const signature = JSON.stringify([s.hs, s.hst, s.o?.net, Math.floor(Date.now() / 60000)]);
    if (signature === lastMachineSignature) return;
    lastMachineSignature = signature;
    const hosts = safeArray(s.hs), live = s.hst || {};
    if (!hosts.length) { byId("aoMachines").innerHTML = '<p class="ao-empty">Ainda não existem máquinas configuradas.</p>'; return; }
    byId("aoMachines").innerHTML = hosts.slice(0,4).map((h)=>{
      const info = live[h.id] || {};
      const enabled = !!h.on_;
      const unavailable = enabled && s.o?.net?.[h.id] === false;
      const age = Date.now() - Number(info.t || 0);
      const recent = enabled && !unavailable && age >= 0 && age < 120_000;
      const status = !enabled ? "Desativada" : unavailable ? "Indisponível" : recent ? "Dados recentes" : "Por confirmar";
      let pieces = [];
      if (Number.isFinite(Number(info.cpu)) && Number(info.cpu) > 0) pieces.push(info.cpu + " núcleos");
      if (info.load != null && Number.isFinite(Number(info.load)) && Number(info.load) >= 0) {
        pieces.push("Carga " + Number(info.load).toLocaleString("pt-PT", {maximumFractionDigits:2}));
      }
      if (info.temp != null && Number.isFinite(Number(info.temp))) {
        pieces.push(Number(info.temp).toLocaleString("pt-PT", {maximumFractionDigits:1}) + " °C");
      }
      if (Array.isArray(info.mem) && Number(info.mem[0])>0) {
        const used = (Number(info.mem[0])-Number(info.mem[1]))/1024;
        pieces.push(used.toFixed(1) + " / " + (Number(info.mem[0])/1024).toFixed(1) + " GB RAM");
      }
      if (Array.isArray(info.disk) && Number(info.disk[0])>0) pieces.push(info.disk[1] + " GB livres");
      return '<div class="ao-machine-row"><span class="ao-host-dot ' + (recent ? "ao-host-up" : "ao-host-down") + '"></span><span><strong>' +
        escapeHTML(h.id) + '</strong><small>' + escapeHTML(pieces.join(" · ") || "Sem métricas de hardware") + '</small></span><em>' +
        status + '</em></div>';
    }).join("");
  };
  const render = (s) => {
    if (!s || typeof s !== "object" || s.error) return;
    snapshot = s;
    lastStateAt = Date.now();
    app.classList.remove("ao-data-stale");
    const isPaused = paused(s), isOffline = offline(s), isWarn = !!s.o?.warn;
    const state = byId("aoOfficeStatus");
    state.className = "ao-status-chip" + (isPaused || isWarn ? " is-paused" : "");
    const statusLabel = isOffline ? "Serviços externos indisponíveis" : isPaused ? "Escritório em pausa" : isWarn ? "Aviso de utilização" : "Ligação ativa";
    if (state.textContent !== statusLabel) state.textContent = statusLabel;
    const connection = isOffline ? "Problema em serviços externos" : "Dados em tempo real";
    if (byId("aoConnection").textContent !== connection) byId("aoConnection").textContent = connection;
    byId("aoPause").textContent = s.o?.man ? "Retomar escritório" : "Pausar escritório";
    byId("aoPause").disabled = !s.o;
    updateAgents(s); renderPipeline(s); renderRepos(s); renderLists(s); renderMerges(s); renderMachines(s);
    if (selected) showAgent(selected, false);
  };
  const overlay = byId("aoAgentOverlay");
  const showAgent = (role, focus = true) => {
    if (!snapshot || !roles.includes(role)) return;
    const w = snapshot.w?.[role] || {}, stage = mode(snapshot,role);
    selected = role;
    byId("aoDetailTitle").textContent = names[role];
    const jobs = safeArray(snapshot.q);
    const job = (w.job != null ? jobs.find((j) => String(j.id) === String(w.job)) : null)
      || jobs.find((j) => workerStages[role]?.includes(j.st));
    const recent = lastCommands(w);
    const signature = JSON.stringify([role, stage, w.md, w.tk, w.say, w.mood, recent, job?.id, job?.title]);
    if (!focus && signature === lastDetailSignature) return;
    lastDetailSignature = signature;
    const rows = [
      ["Estado",statusText[stage]],
      ["Modelo",w.md ? A.mdn(w.md) : "Sem modelo atribuído"],
      ["Tokens hoje",formatNum(w.tk)],
      ["Atividade",w.say || "Sem informação adicional"]
    ];
    if (Object.hasOwn(resultText, w.mood)) rows.push(["Resultado recente", resultText[w.mood].replace("Último resultado: ", "")]);
    if (job) rows.push(["Tarefa",job.title]);
    byId("aoDetailBody").replaceChildren();
    for (const [title,value] of rows) {
      const row = document.createElement("div"), t = document.createElement("span"), v = document.createElement("strong");
      row.className = "ao-detail-row"; t.textContent = title; v.textContent = value;
      row.append(t,v); byId("aoDetailBody").appendChild(row);
    }
    if (recent.length) {
      const section = document.createElement("section"), heading = document.createElement("h3"), list = document.createElement("ol");
      section.className = "ao-command-section";
      heading.textContent = "Comandos recentes";
      list.className = "ao-command-list";
      for (const [when, cmd] of recent) {
        const item = document.createElement("li"), time = document.createElement("time"), label = document.createElement("span");
        time.textContent = datum(when);
        label.textContent = cmd.slice(0, 160);
        item.append(time,label);
        list.appendChild(item);
      }
      section.append(heading,list);
      byId("aoDetailBody").appendChild(section);
    }
    const jb = byId("aoDetailJob"), mb = byId("aoDetailModel");
    jb.hidden = !job || typeof A.job !== "function";
    jb.onclick = () => { overlay.hidden = true; selected = null; A.job(job.id); };
    mb.hidden = ["ci","submit"].includes(role) || typeof A.role !== "function";
    mb.onclick = () => { overlay.hidden = true; selected = null; A.role(role); };
    overlay.hidden = false;
    if (focus) { lastFocus = document.activeElement; overlay.querySelector(".ao-agent-dialog").focus(); }
  };
  const hideAgent = () => {
    if (overlay.hidden) return;
    overlay.hidden = true; selected = null;
    if (lastFocus?.isConnected) lastFocus.focus();
  };
  byId("aoDetailClose").addEventListener("click", hideAgent);
  overlay.addEventListener("click", (e) => { if (e.target === overlay) hideAgent(); });
  document.addEventListener("keydown", (e) => {
    if (overlay.hidden) return;
    if (e.key === "Escape") { e.preventDefault(); hideAgent(); return; }
    if (e.key !== "Tab") return;
    const items = Array.from(overlay.querySelectorAll("button:not([hidden]):not(:disabled)"));
    if (!items.length) { e.preventDefault(); return; }
    const first = items[0], last = items[items.length - 1];
    if (e.shiftKey && (document.activeElement === first || document.activeElement === overlay.querySelector(".ao-agent-dialog"))) {
      e.preventDefault(); last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault(); first.focus();
    }
  });
  const handlers = {repos:"bRepos",hosts:"bHosts",costs:"bCost",settings:"bSet"};
  app.addEventListener("click", (event) => {
    const node = event.target.closest("[data-role],[data-agent],[data-job],[data-pr-url],[data-target],[data-open]");
    if (!node) return;
    if (node.dataset.role || node.dataset.agent) { showAgent(node.dataset.role || node.dataset.agent); return; }
    if (node.dataset.job) { A.job?.(Number(node.dataset.job)); return; }
    if (node.dataset.prUrl) { A.open?.(node.dataset.prUrl); return; }
    if (node.dataset.open) { byId(handlers[node.dataset.open])?.click(); return; }
    if (node.dataset.target) {
      const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches;
      byId(node.dataset.target)?.scrollIntoView({
        behavior: animationsOn && !reduced ? "smooth" : "auto", block: "start"
      });
      app.querySelectorAll(".ao-nav-item").forEach((n) => n.classList.toggle("is-active", n === node));
    }
  });
  byId("aoPause").addEventListener("click", () => byId("bPause")?.click());
  let healthy = true;
  let unsubscribe = () => {};
  let staleTimer = null;
  const startedAt = Date.now();
  const fallback = (error) => {
    healthy = false;
    unsubscribe();
    if (staleTimer !== null) clearInterval(staleTimer);
    console.error("Apollo Control Room: reverting to original interface", error);
    app.remove();
    legacy.replaceWith(...Array.from(legacy.children));
    document.body.classList.remove("ao-enabled");
    A.modernRoom = false;
  };
  // Keep legacy onState intact. State subscribers are isolated by o.js.
  unsubscribe = A.subscribeState((s) => {
    if (!healthy) return;
    try { render(s); } catch (error) { fallback(error); }
  });
  if (A.S) {
    try { render(A.S); } catch (error) { fallback(error); }
  }
  // Keep the connection label honest if WebSocket and polling stop updating.
  staleTimer = setInterval(() => {
    if (!healthy || !app.isConnected) { clearInterval(staleTimer); return; }
    if (Date.now() - (lastStateAt || startedAt) <= 12000) return;
    const state = byId("aoOfficeStatus");
    app.classList.add("ao-data-stale");
    state.className = "ao-status-chip is-paused";
    if (state.textContent !== "Dados desatualizados") state.textContent = "Dados desatualizados";
    byId("aoConnection").textContent = "Sem atualizações recentes";
  }, 5000);
})();
