/* Apollo Office Control Room
 * Presentation-only enhancement. Reads AO.S; never fabricates work or modifies jobs.
 */
(() => {
  "use strict";
  const A = window.AO;
  // O ApolloOffice personalizado mantém o escritório pixelizado por omissão.
  // A vista moderna só é carregada por pedido explícito: ?ui=modern.
  const requestedUi = new URLSearchParams(window.location.search).get("ui");
  if (requestedUi !== "modern") return;
  if (!A || document.querySelector(".ao-app")) return;
  const byId = (id) => document.getElementById(id);
  const escapeHTML = (value) => A.esc(value ?? "");
  const names = A.NM || {};
  const roles = ["scout", "guard", "vet", "watcher", "hunter", "repro", "fixer", "ci", "reviewer", "submit"];
  // The panorama has ten positions, matching the ten real worker roles.
  const officeRoles = roles;
  const colors = ["#3ac9b6", "#5e91ef", "#a283e8", "#4dbee4", "#d1aa6c", "#ef7b6b", "#f39963", "#8792a8", "#d77cb3", "#69c97c"];
  // Role-specific verb shown while a worker is really in the "work" state.
  const workVerb = {scout:"A analisar", guard:"A validar", vet:"Em triagem", watcher:"A monitorizar", hunter:"A pesquisar", repro:"A reproduzir", fixer:"A programar", ci:"A testar", reviewer:"A rever", submit:"A preparar"};
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
  const isoDate = (v) => {
    const d = v ? new Date(v) : null;
    return d && !Number.isNaN(d.getTime()) ? d.toISOString() : "";
  };
  // Honest pipeline progress: a job's position through the real stage sequence.
  const stageProgress = {ready:8, vet:16, hunt:30, repro:42, fix:58, ci:72, review:84, gate:92, approve:95, submit:98, done:100, drop:0};
  const greetWord = () => { const h = new Date().getHours(); return h < 13 ? "Bom dia" : h < 20 ? "Boa tarde" : "Boa noite"; };
  const pct = (n) => Math.max(0, Math.min(100, Math.round(Number(n) || 0)));
  const html = [
    '<div class="ao-app" id="aoControlRoom" lang="pt-PT">',
    '  <aside class="ao-sidebar">',
    '    <div class="ao-brand"><span class="ao-logo" aria-hidden="true"><img src="/assets/brand/apollo-mark.svg" alt=""></span><span><strong>Apollo Office</strong><small>Control Center</small></span></div>',
    '    <nav class="ao-navigation" aria-label="Navegação principal">',
    '      <button class="ao-nav-item is-active" type="button" data-target="aoOverview"><img src="/assets/icons/dashboard.svg" alt="" aria-hidden="true"><span>Painel</span></button>',
    '      <button class="ao-nav-item" type="button" data-target="aoRoomPanel"><img src="/assets/icons/agents.svg" alt="" aria-hidden="true"><span>Agentes</span><span class="ao-nav-count" id="aoNavAgents">' + roles.length + '</span></button>',
    '      <button class="ao-nav-item" type="button" data-target="aoTasks"><img src="/assets/icons/tasks.svg" alt="" aria-hidden="true"><span>Tarefas</span><span class="ao-nav-count" id="aoNavTasks" hidden></span></button>',
    '      <button class="ao-nav-item" type="button" data-target="aoPulls"><img src="/assets/icons/pulls.svg" alt="" aria-hidden="true"><span>Pull Requests</span><span class="ao-nav-count" id="aoNavPulls" hidden></span></button>',
    '      <button class="ao-nav-item" type="button" data-open="repos"><img src="/assets/icons/folder.svg" alt="" aria-hidden="true"><span>Repositórios</span></button>',
    '      <button class="ao-nav-item" type="button" data-target="aoRepos"><img src="/assets/icons/projects.svg" alt="" aria-hidden="true"><span>Projetos</span></button>',
    '      <button class="ao-nav-item" type="button" data-open="hosts"><img src="/assets/icons/machines.svg" alt="" aria-hidden="true"><span>Máquinas</span></button>',
    '      <button class="ao-nav-item" type="button" data-open="costs"><img src="/assets/icons/costs.svg" alt="" aria-hidden="true"><span>Custos</span></button>',
    '      <button class="ao-nav-item" type="button" data-target="aoActivity"><img src="/assets/icons/activity.svg" alt="" aria-hidden="true"><span>Logs</span></button>',
    '      <button class="ao-nav-item" type="button" data-open="settings"><img src="/assets/icons/settings.svg" alt="" aria-hidden="true"><span>Definições</span></button>',
    '    </nav>',
    '    <div class="ao-sidebar-section"><div class="ao-sidebar-section-title"><span>Projetos ativos</span><button class="ao-project-add" type="button" data-open="repos" aria-label="Gerir projetos" title="Gerir projetos">+</button><span id="aoProjectCount" class="sr-only">—</span></div><div id="aoRepos" class="ao-repo-list"><div class="ao-muted">A carregar…</div></div></div>',
    '    <div class="ao-sidebar-footer"><span class="ao-presence-dot"></span><span><strong id="aoConnection">A estabelecer ligação…</strong><small id="aoUpdatedAt" aria-label="Último estado recebido">A aguardar dados</small></span></div>',
    '    <div class="ao-user-card"><span class="ao-user-avatar" id="aoUserInitials" aria-hidden="true">·</span><span class="ao-user-meta"><strong id="aoUserName">Utilizador</strong><small><i class="ao-user-dot" aria-hidden="true"></i>Online</small></span></div>',
    '  </aside>',
    '  <div class="ao-workspace">',
    '    <div class="ao-pixel-toolbar">',
    '      <nav class="ao-pixel-nav" aria-label="Navegação principal">',
    '        <button class="ao-nav-item is-active" type="button" data-target="aoOverview">Painel</button><button class="ao-nav-item" type="button" data-target="aoRoomPanel">Agentes</button><button class="ao-nav-item" type="button" data-target="aoTasks">Tarefas</button><button class="ao-nav-item" type="button" data-target="aoPulls">Pull Requests</button><button class="ao-nav-item" type="button" data-target="aoActivity">Atividade</button>',
    '      </nav>',
    '      <div class="ao-pixel-tools"><label for="aoRepoFilter">Projeto</label><select id="aoRepoFilter" aria-label="Filtrar por projeto"><option value="">Todos os projetos</option></select><button type="button" data-open="costs">Custos</button><button type="button" data-open="repos">Repositórios</button><button type="button" data-open="hosts">Máquinas</button><button type="button" data-open="settings">Definições</button></div>',
    '    </div>',
    '    <header class="ao-topbar"><div class="ao-top-heading"><strong id="aoGreeting">Olá</strong><span>Central de agentes de IA · Desenvolvimento · Revisão · Automação</span></div>',
    '      <div class="ao-top-actions"><span id="aoOfficeStatus" class="ao-status-chip" role="status" aria-live="polite">A ligar…</span><button id="aoPause" type="button" class="ao-outline-button ao-pause-button" disabled aria-label="Pausar escritório" title="Pausar escritório"><span class="ao-pause-glyph" aria-hidden="true"></span><span id="aoPauseLabel">Pausar escritório</span></button><button id="aoBell" type="button" class="ao-icon-button ao-bell" aria-label="Notificações"><span class="ao-bell-dot" hidden></span>!</button><div class="ao-clock" aria-hidden="true"><strong id="aoClock">--:--</strong><small id="aoDate"></small></div></div>',
    '    </header>',
    '    <main class="ao-content" id="aoOverview">',
    '      <section id="aoDiag" class="ao-diag" aria-label="Diagnóstico do ambiente" aria-live="polite" hidden></section>',
    '      <section class="ao-metrics" aria-label="Indicadores operacionais">',
    '        <div class="ao-metric" data-tone="cyan"><img src="/assets/icons/agents.svg" alt="" aria-hidden="true"><span><small>Agentes ativos</small><strong id="aoMetricWorkers">—</strong></span><span id="aoMetricWorkersSub" class="ao-metric-sub" hidden></span></div>',
    '        <div class="ao-metric" data-tone="blue"><img src="/assets/icons/tasks.svg" alt="" aria-hidden="true"><span><small>Tarefas em aberto · Total registado no sistema</small><strong id="aoMetricQueue">—</strong></span><span id="aoMetricQueueBar" class="ao-metric-track" hidden><i></i></span></div>',
    '        <div class="ao-metric" data-tone="violet"><img src="/assets/icons/pulls.svg" alt="" aria-hidden="true"><span><small>PRs abertos (recentes) · Até 40 PRs registados</small><strong id="aoMetricPR">—</strong></span><span id="aoMetricPRSub" class="ao-metric-sub" hidden></span></div>',
    '        <div class="ao-metric" data-tone="green"><img src="/assets/icons/complete.svg" alt="" aria-hidden="true"><span><small>Correções integradas</small><strong id="aoMetricDone">—</strong></span><span class="ao-metric-sub">Relatório do steward</span></div>',
    '      </section>',
    '      <section class="ao-primary-grid" aria-label="Escritório e estado dos agentes">',
    '      <section class="ao-panel ao-room-panel" id="aoRoomPanel" aria-labelledby="aoRoomTitle">',
    '        <div class="ao-panel-heading"><img class="ao-panel-icon" src="/assets/icons/machines.svg" alt="" aria-hidden="true"><div><h2 id="aoRoomTitle">Escritório Virtual</h2><p>Dez papéis · estados observados em tempo real</p></div>',
    '          <div class="ao-room-controls"><span id="aoOfficeMode" class="ao-room-live" aria-live="polite">A aguardar estado</span><span class="ao-room-view" aria-hidden="true">Vista: Escritório</span><label class="ao-motion-label" for="aoMotion">Animações</label><input id="aoMotion" type="checkbox" checked aria-label="Ativar animações" title="A preferência do sistema para movimento reduzido é respeitada"><button id="aoExpand" type="button" class="ao-icon-button" aria-label="Ampliar o escritório" aria-controls="aoRoomPanel" aria-expanded="false" title="Ampliar o escritório">⤢</button></div>',
    '        </div>',
    '        <div class="ao-office" role="group" aria-label="Escritório virtual com dez agentes">',
    '          <div class="ao-office-brand" aria-hidden="true"><img src="/assets/brand/apollo-office-lockup.webp" alt=""><span>AI Agents Lab</span></div>',
    '          <div class="ao-office-check" id="aoOfficeCheck" role="group" aria-label="Tarefas em aberto por fase"><span data-phase="develop"><i aria-hidden="true"></i><span>Desenvolver</span><b class="ao-check-count">—</b></span><span data-phase="test"><i aria-hidden="true"></i><span>Testar</span><b class="ao-check-count">—</b></span><span data-phase="review"><i aria-hidden="true"></i><span>Rever</span><b class="ao-check-count">—</b></span><span data-phase="integrate"><i aria-hidden="true"></i><span>Integrar</span><b class="ao-check-count">—</b></span></div>',
    '          <div class="ao-stations" id="aoStations"></div><div id="aoTransfer" class="ao-transfer-label" role="status" aria-live="polite" hidden></div>',
    '        </div>',
    '      </section>',
    '      <article class="ao-panel ao-agent-panel"><div class="ao-panel-heading"><div><h2>Estado dos agentes</h2><p id="aoAgentSummary">10 papéis</p></div></div><div id="aoAgentList" class="ao-agent-list"></div></article>',
    '      </section>',
    '      <section class="ao-panel ao-pipeline-panel" aria-label="Estado do fluxo de trabalho">',
    '        <div class="ao-panel-heading"><div><h2>Fluxo de trabalho</h2><p>Contagem total de tarefas por fase</p></div></div><div id="aoPipeline" class="ao-pipeline"></div>',
    '      </section>',
    '      <section class="ao-panel ao-staged-panel" id="aoStagedPanel" aria-labelledby="aoStagedTitle" hidden><div class="ao-staged-head"><div><h3 id="aoStagedTitle">Pipeline test-first</h3><p id="aoStagedModels">A ler configuração do controlador…</p></div><span id="aoStagedStatus" class="ao-staged-status" role="status" aria-live="polite">A aguardar dados</span></div><ol id="aoStagedPhases" class="ao-staged-phases" aria-label="Fases do pipeline staged"></ol><p id="aoStagedFeedback" class="ao-staged-feedback" hidden></p></section>',
    '      <section class="ao-operations-grid" aria-label="Operações prioritárias">',
    '        <article class="ao-panel ao-task-panel" id="aoTasks" tabindex="-1"><div class="ao-panel-heading queue-heading"><div class="ao-panel-title"><h2>Tarefas em execução</h2><span id="aoTaskSummary" class="ao-section-count">—</span></div><input class="ao-task-filter" id="aoTaskFilter" type="search" placeholder="Filtrar tarefas" aria-label="Filtrar tarefas por texto"><button class="ao-panel-action" type="button" data-open-list="tasks">Ver todas <span aria-hidden="true">→</span></button></div><div id="aoTasksList" class="ao-list"></div>',
    '        </article>',
    '        <article class="ao-panel" id="aoPulls" tabindex="-1"><div class="ao-panel-heading"><div class="ao-panel-title"><h2>Pull Requests</h2><span id="aoPRSummary" class="ao-section-count">—</span></div><button class="ao-panel-action" type="button" data-open-list="prs">Ver todas <span aria-hidden="true">→</span></button></div><div id="aoMergeChart" class="ao-pr-trend" role="img" aria-label="Integrações dos últimos 30 dias"></div><div id="aoPRList" class="ao-list"></div></article>',
    '        <article class="ao-panel" id="aoActivity" tabindex="-1"><div class="ao-panel-heading"><img class="ao-panel-icon" src="/assets/icons/activity-wave.svg" alt="" aria-hidden="true"><div class="ao-panel-title"><h2>Atividade recente</h2><span id="aoActivitySummary" class="ao-section-count">—</span></div><button class="ao-panel-action" type="button" data-open-list="activity">Ver todo o histórico <span aria-hidden="true">→</span></button></div><div id="aoEventList" class="ao-list"></div></article>',
    '        <article class="ao-panel ao-steward-panel"><div class="ao-panel-heading"><div class="ao-panel-title"><h2>Fila de revisão de PRs</h2><span id="aoStewardSummary" class="ao-section-count">—</span></div><button class="ao-panel-action" type="button" data-open="steward">Abrir fila completa do steward <span aria-hidden="true">→</span></button></div><div id="aoIssues" class="ao-list ao-steward-list"></div></article>',
    '      </section>',
    '      <section class="ao-bottom-grid" aria-label="Métricas adicionais">',
    '        <article class="ao-panel"><div class="ao-panel-heading"><div><h2>Produtividade (24h)</h2><p>Tarefas concluídas por hora</p></div><span class="ao-data-source">Tempo real</span></div><div class="ao-trend" id="aoProductivityChart"></div></article>',
    '        <article class="ao-panel"><div class="ao-panel-heading"><div><h2>Recursos do Sistema</h2><p>Telemetria das máquinas</p></div><span id="aoResourcesHost" class="ao-data-source" hidden></span></div><div id="aoMachines" class="ao-machines"></div></article>',
    '      </section>',
    '    </main>',
    '  </div>',
    '  <div class="ao-agent-overlay" id="aoAgentOverlay" hidden>',
    '    <section class="ao-agent-dialog" role="dialog" aria-modal="true" aria-labelledby="aoDetailTitle" tabindex="-1">',
    '      <button class="ao-dialog-close" id="aoDetailClose" type="button" aria-label="Fechar detalhes">×</button>',
    '      <div class="ao-agent-head"><img id="aoDetailAvatar" class="ao-agent-avatar" alt="" aria-hidden="true"><h2 id="aoDetailTitle"></h2></div><div id="aoDetailBody"></div>',
    '      <div class="ao-detail-actions"><button id="aoDetailJob" type="button" hidden>Ver tarefa</button><button id="aoDetailModel" type="button" hidden>Configurar modelo</button></div>',
    '    </section>',
    '  </div>',
    '  <div class="ao-agent-overlay ao-list-overlay" id="aoListOverlay" hidden>',
    '    <section class="ao-agent-dialog ao-list-dialog" role="dialog" aria-modal="true" aria-labelledby="aoListDialogTitle" tabindex="-1">',
    '      <button class="ao-dialog-close" id="aoListClose" type="button" aria-label="Fechar lista">×</button>',
    '      <div class="ao-list-dialog-head"><img id="aoListDialogIcon" src="/assets/icons/tasks.svg" alt="" aria-hidden="true"><div><h2 id="aoListDialogTitle">Lista completa</h2><span id="aoListDialogCount" class="ao-section-count">—</span></div></div>',
    '      <div id="aoListDialogContent" class="ao-list-dialog-content"></div>',
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
  const pixelStage = byId("stage");
  const pixelCanvas = byId("cv");
  const pixelMerge = byId("mg");
  const pixelTip = byId("tip");
  const pixelTipParent = pixelTip?.parentElement || null;
  const pixelTipNext = pixelTip?.nextSibling || null;
  const pixelOffice = app.querySelector(".ao-office");
  if (pixelCanvas && pixelOffice && pixelStage?.contains(pixelCanvas)) pixelOffice.append(pixelCanvas);
  if (pixelOffice && pixelMerge && pixelStage?.contains(pixelMerge)) pixelOffice.append(pixelMerge);
  if (pixelTip) document.body.append(pixelTip);
  A.pixelDashboard = true;
  A.modernRoom = false;
  A.pixelMotion = true;
  A.disablePixelArt = false;

  const stations = byId("aoStations");
  stations.innerHTML = officeRoles.map((role) => {
    const i = roles.indexOf(role);
    return '<button class="ao-station" type="button" data-role="' + role + '" data-mode="unknown" style="--agent-accent:' + colors[i] + '" aria-label="Ver agente ' + escapeHTML(names[role]) + ' · sem dados">' +
      '<span class="ao-plaque"><span class="ao-station-name">' + escapeHTML(names[role]) + '</span>' +
      '<span class="ao-station-caption"><i class="ao-station-indicator" aria-hidden="true"></i><span class="ao-station-state">A ligar…</span><span class="ao-result-mark" aria-hidden="true"></span></span></span>' +
      '</button>';
  }).join("");
  const agentList = byId("aoAgentList");
  agentList.innerHTML = roles.map((role, i) =>
    '<button class="ao-agent-row" type="button" data-role="' + role + '" aria-label="Ver agente ' + escapeHTML(names[role]) + '">' +
      '<i class="ao-agent-color" style="background:' + colors[i] + '" aria-hidden="true"></i>' +
      '<span class="ao-agent-meta"><strong class="ao-agent-title">' + escapeHTML(names[role]) + '</strong><small class="ao-agent-activity">Sem atividade registada</small><small class="ao-agent-model" hidden></small></span>' +
      '<span class="ao-agent-status">Sem dados</span></button>'
  ).join("");
  const taskFilterInput = byId("aoTaskFilter");
  window.AOOfficeV2?.decorate(document.querySelector('.ao-office'), stations);
  const stationNodes = Object.fromEntries(officeRoles.map((role) => [role, stations.querySelector('[data-role="' + role + '"]')]));
  // Arrow navigation follows the station grid; buttons remain individually tabbable.
  stations.addEventListener("keydown", (event) => {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    const station = event.target.closest?.("[data-role]");
    if (!station || !stations.contains(station)) return;
    const index = officeRoles.indexOf(station.dataset.role);
    if (index < 0) return;
    const columns = window.matchMedia("(max-width: 600px)").matches ? 2 : 5;
    const offset = {ArrowLeft: -1, ArrowRight: 1, ArrowUp: -columns, ArrowDown: columns}[event.key];
    if (!offset) return;
    const next = index + offset;
    if (next < 0 || next >= officeRoles.length) return;
    event.preventDefault();
    stationNodes[officeRoles[next]].focus({preventScroll:true});
  });
  // One-shot visual reactions are cleared when their real state is no longer current.
  // Transient state highlights are cleared on stale data, hidden tabs, or reduced motion.
  /**
   * Limpa animações transitórias quando o movimento é suspenso,
   * o documento fica oculto ou os dados deixam de ser recentes.
   */
  const clearReactions = () => {
    // State-driven event highlights must not outlive their real data.
    A.cancelOfficeMotion?.();
    window.AOOfficeV2?.clearTransitions(stations);
    for (const node of Object.values(stationNodes)) {
      node.classList.remove("ao-agent-state-change", "ao-stage-arrived", "ao-celebrate", "ao-action-pulse");
    }
  };
  for (const node of Object.values(stationNodes)) {
    node.addEventListener("animationend", (event) => {
      if (event.animationName === "ao-celebrate") node.classList.remove("ao-celebrate");
      if (event.animationName === "ao-command-sweep") node.classList.remove("ao-action-pulse");
      if (event.animationName === "ao-event-pop") {
        node.classList.remove("ao-agent-state-change", "ao-stage-arrived");
      }
    });
  }
  let snapshot = null, selected = null, selectedRepo = "", lastFocus = null;
  let taskFilterText = "";
  let lastRepoSignature = "", lastListSignature = "", lastMergeSignature = "", lastProdSignature = "", lastPipelineSignature = "";
  let listDialogData = {tasks: "", prs: "", activity: ""};
  let lastMachineSignature = "", lastDetailSignature = "", lastStateAt = 0;
  let animationsOn = true;
  try { animationsOn = localStorage.getItem("ao.motion") !== "off"; } catch {}
  const motion = byId("aoMotion");
  motion.checked = animationsOn;
  const syncMotion = () => {
    app.classList.toggle("ao-motion-disabled", !animationsOn);
    A.pixelMotion = animationsOn;
  };
  syncMotion();
  motion.addEventListener("change", () => {
    animationsOn = motion.checked;
    try { localStorage.setItem("ao.motion", animationsOn ? "on" : "off"); } catch {}
    syncMotion();
    if (!animationsOn) clearReactions();
  });
  const onVisibilityChange = () => {
    if (document.hidden) clearReactions();
  };
  document.addEventListener("visibilitychange", onVisibilityChange);
  byId("aoExpand").addEventListener("click", (e) => {
    const expanded = byId("aoRoomPanel").classList.toggle("ao-expanded");
    e.currentTarget.setAttribute("aria-label", expanded ? "Reduzir o escritório" : "Ampliar o escritório");
    e.currentTarget.title = expanded ? "Reduzir o escritório" : "Ampliar o escritório";
    e.currentTarget.setAttribute("aria-expanded", String(expanded));
  });
  const offline = (s) => Boolean(s?.o?.off);
  // External provider/network failures are not the same as manual pausing;
  // a locally running worker can still be active while o.off is true.
  const paused = (s) => Boolean(s?.o?.man || s?.o?.pause);
  // The server already normalizes persistent background jobs into w[role].st
  // (web.rs::state). Queue membership alone is not proof of worker activity.
  /**
   * Resolve a worker's visible state from the authoritative backend snapshot.
   * Queued jobs alone are never treated as evidence of active execution.
   * @param {object} s Latest /api/state payload.
   * @param {string} role Worker identifier.
   * @returns {"work"|"idle"|"sleep"|"paused"|"unknown"}
   */
  const mode = (s, role) => {
    if (!s?.w || !Object.hasOwn(s.w, role)) return "unknown";
    const w = s.w[role] || {};
    if (w.hold || paused(s)) return "paused";
    if (w.st === "work") return "work";
    if (w.st === "sleep") return "sleep";
    if (w.st === "idle") return "idle";
    return "unknown";
  };
  const statusText = {
    work:"A trabalhar", idle:"Disponível", sleep:"Em repouso",
    paused:"Em pausa", unknown:"Sem dados"
  };
  const resultText = {ok:"Último resultado: OK",sad:"Último resultado: problema",err:"Último resultado: erro"};
  const lastCommands = (worker) => safeArray(worker.cmds).filter((entry) =>
    Array.isArray(entry) && entry.length >= 2 && typeof entry[1] === "string").slice(-4).reverse();
  const updateAgents = (s) => {
    let working = 0;
    for (const role of roles) {
      const w = s.w?.[role] || {}, el = stationNodes[role], state = mode(s, role);
      if (state === "work") working++;
      const activeJob = w.job;
      const priorJob = el.dataset.lastJob;
      const completed = Number.isFinite(Number(s.t)) && safeArray(s.dn).some((job) => {
        const finishedAt = Number(job.t1), age = Number(s.t) - finishedAt;
        return priorJob && String(job.id) === priorJob && job.st === "done" && job.t1 != null &&
          Number.isFinite(finishedAt) && age >= 0 && age <= 60_000;
      });
      if (el.dataset.mode === "work" && state !== "work" && completed && !paused(s) && animationsOn &&
          !document.hidden && !window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches) {
        el.classList.remove("ao-celebrate");
        void el.offsetWidth;
        el.classList.add("ao-celebrate");
      }
      if (state === "work" && activeJob != null) el.dataset.lastJob = String(activeJob);
      else if (state !== "work") delete el.dataset.lastJob;
      const latestCommand = lastCommands(w)[0];
      if (latestCommand) {
        const commandKey = String(latestCommand[0]) + ":" + latestCommand[1];
        if (el.dataset.lastCommand && el.dataset.lastCommand !== commandKey && state === "work" && animationsOn &&
            !document.hidden && !window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches) {
          el.classList.remove("ao-action-pulse");
          void el.offsetWidth;
          el.classList.add("ao-action-pulse");
        }
        el.dataset.lastCommand = commandKey;
      }
      const row = agentList.querySelector('[data-role="' + role + '"]');
      if (row) {
        const job = w.job != null ? safeArray(s.q).find((item) => String(item.id) === String(w.job)) : null;
        const activity = row.querySelector(".ao-agent-activity");
        const model = row.querySelector(".ao-agent-model");
        const stateNode = row.querySelector(".ao-agent-status");
        const current = typeof w.say === "string" ? w.say.trim() : "";
        const detail = current ? (state === "work" ? current : "Último registo: " + current)
          : state === "work" ? job?.title || "Sem descrição da tarefa" : "Sem atividade registada";
        row.dataset.state = state;
        row.classList.toggle("ao-state-work", state === "work");
        row.classList.toggle("ao-state-paused", state === "paused");
        activity.textContent = detail;
        activity.title = detail;
        stateNode.textContent = state === "work" ? (workVerb[role] || statusText[state]) : statusText[state];
        const modelText = w.md && !["ci", "submit"].includes(role) && typeof A.mdn === "function" ? String(A.mdn(w.md)) : "";
        model.hidden = !modelText;
        model.textContent = modelText;
        row.setAttribute("aria-label", ["Ver agente " + names[role], statusText[state], detail, w.md && A.mdn(w.md)].filter(Boolean).join(". "));
      }
      if (!el) continue;
      window.AOOfficeV2?.onAgentState(el, el.dataset.mode, state);
      el.dataset.mode = state;
      const result = Object.hasOwn(resultText, w.mood) ? w.mood : "";
      el.dataset.mood = result;
      el.querySelector(".ao-result-mark").textContent = result === "ok" ? "✓" : result === "err" ? "!" : result === "sad" ? "!" : "";
      const stage = w.job != null ? safeArray(s.q).find((job) => String(job.id) === String(w.job))?.st : null;
      const stateLabel = statusText[state];
      // Live state → the role's work verb when really working, else the generic state.
      el.querySelector(".ao-station-state").textContent = state === "work" ? (workVerb[role] || stateLabel) : stateLabel;
      el.title = [names[role], stateLabel, stage && (stageLabels[stage] || stage), resultText[result], w.say].filter(Boolean).join(" · ");
      const accessible = ["Ver agente " + names[role], statusText[state], resultText[result]].filter(Boolean).join(". ");
      if (el.getAttribute("aria-label") !== accessible) el.setAttribute("aria-label", accessible);
    }
    byId("aoMetricWorkers").textContent = formatNum(working) + " / " + roles.length;
    byId("aoAgentSummary").textContent = formatNum(working) + " em execução · " + formatNum(roles.length - working) + " restantes";
  };
  const rowHTML = (label, caption, badge, attr, val, opts = {}) => {
    const {extraClass = "", progress = null, badgeTone = ""} = opts;
    const unavailable = attr === "data-pr-url" && !val;
    const bar = progress == null ? "" :
      '<span class="ao-row-progress" aria-hidden="true"><i style="width:' + pct(progress) + '%"></i></span>' +
      '<span class="ao-row-pct">' + pct(progress) + '%</span>';
    return '<button class="ao-data-row ' + extraClass + (progress == null ? "" : " has-progress") + '" type="button" ' +
      (unavailable ? 'disabled title="Ligação indisponível"' : attr + '="' + escapeHTML(val) + '"') + '>' +
      '<span class="ao-row-main"><strong>' + escapeHTML(label) + '</strong><small>' + escapeHTML(caption) + '</small></span>' +
      bar +
      '<span class="ao-row-badge ' + badgeTone + '">' + escapeHTML(badge) + '</span></button>';
  };
  const taskIcon = (stage) => ({
    hunt: "checklist.svg", vet: "checklist.svg", repro: "code.svg", fix: "code.svg",
    ci: "flask.svg", review: "pulls.svg", gate: "pulls.svg", approve: "pulls.svg", submit: "pulls.svg"
  }[stage] || "tasks.svg");
  const projectName = (repo) => String(repo || "Projeto").split("/").filter(Boolean).pop() || "Projeto";
  const taskRowHTML = (job) => {
    const progress = stageProgress[job.st] ?? null;
    const bar = progress == null ? "" :
      '<span class="ao-row-progress" aria-label="' + pct(progress) + '% concluído"><i style="width:' + pct(progress) + '%"></i></span>' +
      '<span class="ao-row-pct">' + pct(progress) + '%</span>';
    return '<button class="ao-data-row ao-task-row" type="button" data-job="' + escapeHTML(job.id) + '">' +
      '<img class="ao-task-kind" src="/assets/icons/' + taskIcon(job.st) + '" alt="" aria-hidden="true">' +
      '<span class="ao-task-title" title="' + escapeHTML(job.title) + '">' + escapeHTML(job.title) + '</span>' +
      '<span class="ao-project-chip" title="' + escapeHTML(job.repo) + '"><i aria-hidden="true"></i>' + escapeHTML(projectName(job.repo)) + '</span>' +
      bar + '</button>';
  };
  // Map a PR to a real status badge, cross-referencing the live job stage.
  const prBadge = (s, p) => {
    if (p.st === "merged") return {t: "Integrado", tone: "ao-badge-ok"};
    if (p.st === "closed") return {t: "Fechado", tone: ""};
    const job = safeArray(s.q).find((j) => j.repo === p.repo && String(j.num) === String(p.num));
    if (job) {
      if (job.st === "review") return {t: "Em revisão", tone: "ao-badge-review"};
      if (job.st === "ci") return {t: "Testes", tone: "ao-badge-wait"};
      if (["approve", "gate", "submit"].includes(job.st)) return {t: "Pronto", tone: "ao-badge-ok"};
    }
    return {t: "Aberto", tone: "ao-badge-open"};
  };
  const pullRowHTML = (s, p) => {
    const badge = prBadge(s, p);
    const commentValue = p.comments_count ?? p.comment_count ?? p.comments;
    const commentCount = Array.isArray(commentValue) ? commentValue.length : Number(commentValue);
    const comments = commentValue != null && Number.isFinite(commentCount)
      ? '<span class="ao-pr-comments" aria-label="' + formatNum(commentCount) + ' comentários"><img src="/assets/icons/comment.svg" alt="" aria-hidden="true">' + formatNum(commentCount) + '</span>' : "";
    return '<button class="ao-data-row ao-pr-row" type="button" ' + (p.url ? 'data-pr-url="' + escapeHTML(p.url) + '"' : 'disabled title="Ligação indisponível"') + '>' +
      '<img class="ao-pr-mark" src="/assets/icons/github.svg" alt="" aria-hidden="true">' +
      '<span class="ao-pr-main"><strong><span class="ao-pr-number">#' + escapeHTML(p.num) + '</span> ' + escapeHTML(p.title) + '</strong><small><i aria-hidden="true"></i>' + escapeHTML(projectName(p.repo)) + '</small></span>' +
      '<span class="ao-row-badge ' + badge.tone + '">' + escapeHTML(badge.t) + '</span>' + comments + '</button>';
  };
  const eventRowHTML = (e) =>
    '<div class="ao-event"><img class="ao-event-mark" src="/assets/icons/complete.svg" alt="" aria-hidden="true"><time datetime="' + escapeHTML(isoDate(e.t)) + '">' + datum(e.t) + '</time><span class="ao-event-agent">' + escapeHTML(names[e.w] || e.w || "Sistema") +
    '</span><span class="ao-event-message" title="' + escapeHTML(e.m) + '">' + escapeHTML(e.m) + '</span></div>';
  const filtered = (items) => safeArray(items).filter((x) => !selectedRepo || x.repo === selectedRepo);
  /**
   * Render the limited task, pull-request and activity previews.
   * Preserves keyboard focus on an equivalent row after live updates.
   * @param {object} s Latest server state.
   */
  const renderLists = (s) => {
    const matchesTask = (job) => !taskFilterText || [job.title, job.repo, job.num, job.kind, job.why, stageLabels[job.st] || job.st]
      .filter(Boolean).join(" ").toLocaleLowerCase("pt-PT").includes(taskFilterText);
    const q = filtered(s.q).filter(matchesTask), prs = filtered(s.pr);
    const totals = countTotals(s);
    const openPRs = prs.filter((x) => x.st === "open");
    const reviewPRs = openPRs.filter((p) => safeArray(s.q).some((j) => j.repo === p.repo && String(j.num) === String(p.num) && j.st === "review"));
    byId("aoMetricQueue").textContent = formatNum(totals.active);
    const qbar = byId("aoMetricQueueBar");
    qbar.hidden = totals.active <= 0;
    if (!qbar.hidden) qbar.firstElementChild.style.width = pct(totals.active / Math.max(6, totals.active) * 100) + "%";
    byId("aoMetricPR").textContent = formatNum(openPRs.length);
    const navTasks = byId("aoNavTasks");
    if (navTasks) { navTasks.hidden = totals.active <= 0; navTasks.textContent = formatNum(totals.active); }
    const navPulls = byId("aoNavPulls");
    if (navPulls) { navPulls.hidden = openPRs.length <= 0; navPulls.textContent = formatNum(openPRs.length); }
    const prSub = byId("aoMetricPRSub");
    prSub.hidden = reviewPRs.length === 0;
    if (!prSub.hidden) prSub.textContent = formatNum(reviewPRs.length) + " em revisão";
    byId("aoMetricDone").textContent = formatNum(s.delivery?.merged_fixes);
    const activeStages = ["hunt", "repro", "fix", "ci", "review", "gate", "submit"];
    const activeTasks = q.filter((job) => activeStages.includes(job.st));
    const approvals = q.filter((job) => job.st === "approve");
    const otherOpenTasks = q.filter((job) => !activeStages.includes(job.st) && !["approve", "done", "drop"].includes(job.st));
    const signature = JSON.stringify([selectedRepo, taskFilterText, q, prs, safeArray(s.ev).slice(0, 8), s.pr_steward]);
    if (signature === lastListSignature) return;
    lastListSignature = signature;
    // A backend update should not unexpectedly reset keyboard navigation.
    // Capture only a focused work item, never an unrelated control.
    const active = document.activeElement;
    const focusedParent = active?.closest?.("#aoTasksList, #aoPRList");
    const focusedAttr = focusedParent && active.matches?.("[data-job], [data-pr-url]")
      ? (active.hasAttribute("data-job") ? "data-job" : "data-pr-url") : null;
    const focusKey = focusedAttr ? {
      parent: focusedParent.id, attribute: focusedAttr,
      value: active.getAttribute(focusedAttr)
    } : null;
    const events = safeArray(s.ev);
    byId("aoTaskSummary").textContent = formatNum(activeTasks.length);
    byId("aoPRSummary").textContent = formatNum(openPRs.length);
    byId("aoActivitySummary").textContent = formatNum(events.slice(0, 8).length);
    const allTaskRows = activeTasks.map(taskRowHTML).concat(
      otherOpenTasks.map((j) => rowHTML(j.title, j.repo + (j.num ? " · #" + j.num : ""), stageLabels[j.st] || j.st, "data-job", j.id, {progress: stageProgress[j.st] ?? null})),
      approvals.map((j) => rowHTML(j.title, j.repo + (j.num ? " · #" + j.num : ""), "Aprovação pendente", "data-job", j.id, {extraClass: "is-approval", badgeTone: "ao-badge-wait"}))
    );
    const allTasksHTML = allTaskRows.join("") || '<p class="ao-empty">Sem tarefas ou decisões pendentes.</p>';
    const allPRsHTML = prs.map((p) => pullRowHTML(s, p)).join("") || '<p class="ao-empty">Sem pull requests registados.</p>';
    const allEventsHTML = events.map(eventRowHTML).join("") || '<p class="ao-empty">Sem atividade recente.</p>';
    listDialogData = {tasks: allTasksHTML, prs: allPRsHTML, activity: allEventsHTML};
    const previewTasks = taskFilterText ? activeTasks.concat(otherOpenTasks, approvals) : activeTasks;
    byId("aoTasksList").innerHTML = previewTasks.slice(0, 4).map((job) =>
      activeStages.includes(job.st) ? taskRowHTML(job)
        : rowHTML(job.title, job.repo + (job.num ? " · #" + job.num : ""), stageLabels[job.st] || (job.st === "approve" ? "Aprovação pendente" : job.st), "data-job", job.id)
    ).join("") ||
      '<p class="ao-empty">Sem tarefas em aberto.</p>';
    byId("aoPRList").innerHTML = openPRs.slice(0, 3).map((p) => pullRowHTML(s, p)).join("") ||
      '<p class="ao-empty">Sem pull requests registados.</p>';
    byId("aoEventList").innerHTML = events.slice(0, 8).map(eventRowHTML).join("") ||
      '<p class="ao-empty">Sem atividade recente.</p>';
    const stewardPRs = safeArray(s.pr_steward?.prs).filter((pr) => !selectedRepo || pr.repo === selectedRepo);
    const stewardRepo = safeArray(s.pr_steward?.repos).find((repo) => repo.repo === selectedRepo);
    const stewardTotalValue = selectedRepo ? stewardRepo?.total : s.pr_steward?.total;
    const stewardBlockedValue = selectedRepo ? stewardRepo?.blocked : s.pr_steward?.blocked;
    const stewardTotal = Number.isFinite(Number(stewardTotalValue)) ? Number(stewardTotalValue) : stewardPRs.length;
    const stewardBlocked = Number.isFinite(Number(stewardBlockedValue)) ? Number(stewardBlockedValue) : stewardPRs.length;
    byId("aoStewardSummary").textContent = formatNum(stewardTotal) + " abertas · " + formatNum(stewardBlocked) + " por tratar";
    byId("aoIssues").innerHTML = stewardPRs.slice(0, 6).map((pr) => rowHTML(
      (pr.num ? "#" + pr.num + " · " : "") + (pr.title || "Pull request"),
      [pr.repo, pr.next_action].filter(Boolean).join(" · "),
      pr.state || "Pendente", "data-pr-url", pr.url,
      {badgeTone: "ao-badge-wait"}
    )).join("") || '<p class="ao-empty">Sem PRs na fila do steward.</p>';
    if (focusKey) {
      const parent = byId(focusKey.parent);
      const next = Array.from(parent.querySelectorAll("button[" + focusKey.attribute + "]"))
        .find((node) => node.getAttribute(focusKey.attribute) === focusKey.value);
      if (next) next.focus({preventScroll:true});
      else byId(focusKey.parent === "aoTasksList" ? "aoTasks" : "aoPulls").focus({preventScroll:true});
    }
  };
  taskFilterInput.addEventListener("input", () => {
    taskFilterText = taskFilterInput.value.trim().toLocaleLowerCase("pt-PT");
    lastListSignature = "";
    if (snapshot) renderLists(snapshot);
  });
  const stagedPanel = byId("aoStagedPanel");
  const stagedModels = byId("aoStagedModels");
  const stagedStatus = byId("aoStagedStatus");
  const stagedPhasesList = byId("aoStagedPhases");
  const stagedFeedback = byId("aoStagedFeedback");
  const renderRepos = (s) => {
    const configured = safeArray(s.rp);
    const repos = [...new Set(configured.map((v) => v.repo).concat(safeArray(s.q), safeArray(s.dn), safeArray(s.pr)).map((v) => typeof v === "string" ? v : v.repo).filter(Boolean))].sort();
    if (selectedRepo && !repos.includes(selectedRepo)) selectedRepo = "";
    const signature = JSON.stringify([repos, configured.map((entry) => [entry.repo, !!entry.on_]), selectedRepo]);
    const projectSelect = byId("aoRepoFilter");
    const optionSignature = JSON.stringify(repos);
    if (projectSelect.dataset.signature !== optionSignature) {
      projectSelect.replaceChildren(new Option("Todos os projetos", ""));
      for (const repo of repos) projectSelect.add(new Option(repo.split("/").at(-1), repo));
      projectSelect.dataset.signature = optionSignature;
    }
    projectSelect.value = selectedRepo;
    if (signature === lastRepoSignature) return;
    lastRepoSignature = signature;
    byId("aoProjectCount").textContent = formatNum(configured.filter((entry) => !!entry.on_).length);
    const wrap = byId("aoRepos");
    const restoreFocus = wrap.contains(document.activeElement);
    wrap.replaceChildren();
    for (const repo of repos) {
      const item = document.createElement("button");
      item.type = "button"; item.className = "ao-repo-item" + (selectedRepo === repo ? " is-selected" : "");
      item.innerHTML = '<span class="ao-repo-dot ' + (configured.find((entry) => entry.repo === repo)?.on_ ? 'is-enabled' : '') + '" aria-hidden="true"></span><span>' + escapeHTML(repo.split("/").at(-1)) + '</span>';
      item.title = repo + (configured.some((entry) => entry.repo === repo && entry.on_) ? " · ativo" : " · configurado / sem atividade");
      item.dataset.repo = repo;
      item.addEventListener("click", () => { selectedRepo = repo; renderRepos(snapshot); renderLists(snapshot); renderPipeline(snapshot); renderOfficeCheck(snapshot); });
      wrap.appendChild(item);
    }
    if (!repos.length) {
      const empty = document.createElement("div");
      empty.className = "ao-muted"; empty.textContent = "Sem repositórios configurados.";
      wrap.appendChild(empty);
    }
    if (restoreFocus) {
      const target = Array.from(wrap.querySelectorAll(".ao-repo-item")).find((node) =>
        node.dataset.repo === selectedRepo);
      target?.focus({preventScroll:true});
    }
  };
  byId("aoRepoFilter").addEventListener("change", (event) => {
    selectedRepo = event.currentTarget.value;
    renderRepos(snapshot);
    renderLists(snapshot);
    renderPipeline(snapshot);
    renderOfficeCheck(snapshot);
  });
  // Unlike the limited queue preview, s.n contains the server's full database counts.
  const pipelineSteps = [
    {label: "Pendente", statuses: ["ready"]},
    {label: "Triagem", statuses: ["vet"]},
    {label: "Pesquisa", statuses: ["hunt"]},
    {label: "Reprodução", statuses: ["repro"]},
    {label: "Correção", statuses: ["fix"]},
    {label: "Testes", statuses: ["ci"]},
    {label: "Revisão", statuses: ["review"]},
    {label: "Validação", statuses: ["gate"]},
    {label: "Aprovação", statuses: ["approve"]},
    {label: "Submissão", statuses: ["submit"]}
  ];
  const officePhases = [
    {key:"develop", label:"Desenvolver", statuses:["ready","vet","hunt","repro","fix"]},
    {key:"test", label:"Testar", statuses:["ci"]},
    {key:"review", label:"Rever", statuses:["review"]},
    {key:"integrate", label:"Integrar", statuses:["gate","approve","submit"]}
  ];
  const pipeline = byId("aoPipeline");
  pipeline.innerHTML = pipelineSteps.map(({label}, i) =>
    '<div class="ao-pipeline-stage" role="group" data-pipeline-index="' + i +
    '"><strong>' + label +
    '</strong><span class="ao-pipeline-value">—</span></div>'
  ).join("");
  /**
   * Present full job totals grouped by the backend's ten distinct statuses.
   * @param {object} s Server state with untruncated s.n status counts.
   */
  const renderPipeline = (s) => {
    const counts = s.n && typeof s.n === "object" ? s.n : {};
    const repoCounts = selectedRepo ? safeArray(s.byRepo).filter((row) => row.repo === selectedRepo)
      .reduce((total, row) => { total[row.st] = Number(row.n) || 0; return total; }, {}) : counts;
    const values = pipelineSteps.map(({statuses}) =>
      statuses.reduce((sum, key) => sum + Math.max(0, Number(repoCounts[key]) || 0), 0)
    );
    const signature = JSON.stringify(values);
    if (signature === lastPipelineSignature) return;
    lastPipelineSignature = signature;
    for (let i = 0; i < values.length; i++) {
      const item = pipeline.children[i];
      item.classList.toggle("is-busy", values[i] > 0);
      item.querySelector(".ao-pipeline-value").textContent = formatNum(values[i]);
      item.setAttribute("aria-label", pipelineSteps[i].label + ": " + formatNum(values[i]) + (values[i] === 1 ? " tarefa" : " tarefas"));
    }
  };
  let lastOfficeCheckSignature = "";
  const renderOfficeCheck = (s) => {
    const counts = s.n && typeof s.n === "object" ? s.n : {};
    const repoCounts = selectedRepo ? safeArray(s.byRepo).filter((row) => row.repo === selectedRepo)
      .reduce((total, row) => { total[row.st] = Number(row.n) || 0; return total; }, {}) : counts;
    const values = officePhases.map(({statuses}) => statuses.reduce((sum, key) =>
      sum + Math.max(0, Number(repoCounts[key]) || 0), 0));
    const signature = JSON.stringify(values);
    if (signature === lastOfficeCheckSignature) return;
    lastOfficeCheckSignature = signature;
    const nodes = byId("aoOfficeCheck").children;
    for (let i = 0; i < values.length; i++) {
      const item = nodes[i], value = values[i], label = officePhases[i].label;
      item.dataset.active = String(value > 0);
      item.querySelector(".ao-check-count").textContent = formatNum(value);
      item.setAttribute("aria-label", label + ": " + formatNum(value) + (value === 1 ? " tarefa" : " tarefas"));
    }
  };
  const stagedPhases = [
    {key:"red", label:"RED · teste regressivo", short:"RED"},
    {key:"green", label:"GREEN · correção", short:"GREEN"},
    {key:"ci", label:"CI · testes", short:"CI"},
    {key:"review", label:"REVIEWER · revisão", short:"REVIEW"},
    {key:"approval", label:"Aprovação humana", short:"Humana"}
  ];
  const stagedRank = {fix:0, ci:1, review:2, gate:3, approve:4, submit:5, done:6};
  const resolveStagedPipeline = (s) => {
    const config = s?.tdd && typeof s.tdd === "object" ? s.tdd : {};
    const repos = safeArray(config.repos).filter((repo) => typeof repo === "string");
    if (config.active !== true || !repos.length) return {visible:false};
    const belongs = (job) => job && repos.includes(job.repo) && ["fix", "hunt"].includes(job.kind);
    const open = safeArray(s.q).filter(belongs);
    const fixer = s.w?.fixer || {};
    const current = open.find((job) => String(job.id) === String(fixer.job));
    const job = current || open[0] || safeArray(s.dn).find((item) => item.st === "drop" && belongs(item));
    if (!job) return {
      visible:true, config, job:null,
      phases:stagedPhases.map((phase) => ({...phase, state:"pending"})),
      status:"À espera de tarefa staged", notice:null
    };

    const workerOwnsJob = String(fixer.job) === String(job.id);
    const latestEvent = safeArray(s.ev).find((event) => String(event?.j) === String(job.id));
    const message = String(latestEvent?.m || (workerOwnsJob ? fixer.say : "") || "").trim();
    const stage = String(job.st || "");
    const rank = Object.hasOwn(stagedRank, stage) ? stagedRank[stage] : -1;
    const redCurrent = stage === "fix" && workerOwnsJob && /^RED(?:\b|:)/i.test(message);
    const greenCurrent = stage === "fix" && workerOwnsJob && /^GREEN(?:\b|\s|:)/i.test(message);
    const phases = stagedPhases.map((phase) => {
      let state = "pending";
      if (phase.key === "red") {
        state = redCurrent ? "active" : (job.red_commit || job.red_stage === "committed" || job.green_commit || rank >= 1) ? "complete" : "pending";
      } else if (phase.key === "green") {
        state = greenCurrent ? "active" : (job.green_commit || rank >= 1) ? "complete" : "pending";
      } else if (phase.key === "ci") {
        state = stage === "ci" ? "active" : rank > 1 ? "complete" : "pending";
      } else if (phase.key === "review") {
        state = stage === "review" ? "active" : rank > 3 ? "complete" : "pending";
      } else if (phase.key === "approval") {
        state = stage === "approve" ? "active" : rank > 4 ? "complete" : "pending";
      }
      return {...phase, state};
    });
    const recovering = /\bresum(?:e|ed|ing)\b|\brecover(?:y|ed)?\b/i.test(message);
    const rejected = stage === "drop" || /\breject(?:ed|ion)?\b|back to fixer|gave up|attempt failed|proof missing|changes requested|reviewer objections/i.test(message);
    const notice = recovering ? {kind:"recovery", label:"Recuperação", message}
      : rejected ? {kind:"rejection", label:"Rejeição", message:String(job.why || message || "Candidato recusado")}
      : null;
    const status = notice?.label || (stage === "approve" ? "Aprovação humana pendente"
      : stage === "submit" ? "Aprovado · a submeter"
      : stage === "review" ? "Revisão independente"
      : stage === "ci" ? "CI em execução"
      : stage === "gate" ? "Validação automática"
      : stage === "fix" ? (greenCurrent ? "GREEN em execução" : redCurrent ? "RED em execução" : "Fixer em execução")
      : stage === "drop" ? "Rejeição"
      : "Tarefa " + stage);
    return {visible:true, config, job, phases, status, notice};
  };
  let lastStagedSignature = "";
  const renderStagedPipeline = (s) => {
    const view = resolveStagedPipeline(s);
    stagedPanel.hidden = !view.visible;
    if (!view.visible) { lastStagedSignature = ""; return; }
    const signature = JSON.stringify(view);
    if (signature === lastStagedSignature) return;
    lastStagedSignature = signature;
    const modelName = (value) => value && typeof A.mdn === "function" ? A.mdn(value) : value || "Sem modelo configurado";
    const primary = view.config.primary || "";
    const reviewer = view.config.reviewer || "";
    const independent = !!primary && !!reviewer && primary !== reviewer;
    const fallback = view.config.fallback
      ? modelName(view.config.fallback) + (view.config.fallback_effort ? " · esforço " + view.config.fallback_effort : " · fallback")
      : "Sem fallback configurado";
    const compactModel = (value) => modelName(value).replace(/\s·\s(?:local|ChatGPT)$/i, "");
    const compactFallback = view.config.fallback
      ? compactModel(view.config.fallback) + (view.config.fallback_effort ? " · esforço " + view.config.fallback_effort : " · fallback")
      : fallback;
    stagedModels.textContent = "FIXER: " + compactModel(primary) + " · Fallback: " + compactFallback +
      " · REVIEWER: " + compactModel(reviewer) + (independent ? " · independente" : " · separação por confirmar");
    stagedStatus.textContent = view.status;
    stagedStatus.dataset.state = view.notice?.kind || (view.job?.st === "approve" ? "approval" : "normal");
    if (!stagedPhasesList.children.length) {
      stagedPhasesList.innerHTML = stagedPhases.map((phase, index) =>
        '<li class="ao-staged-phase" data-stage="' + phase.key + '" title="' + escapeHTML(phase.label) + '"><span class="ao-staged-index">' +
        String(index + 1).padStart(2, "0") + '</span><strong>' + phase.short +
        '</strong><small>À espera</small></li>'
      ).join("");
    }
    const stateLabels = {active:"Em curso", complete:"Concluído", pending:"À espera"};
    for (let i = 0; i < view.phases.length; i++) {
      const phase = view.phases[i], item = stagedPhasesList.children[i];
      item.dataset.state = phase.state;
      item.setAttribute("aria-label", phase.label + ": " + stateLabels[phase.state]);
      item.querySelector("small").textContent = stateLabels[phase.state];
    }
    stagedFeedback.hidden = !view.notice;
    const jobTag = view.job ? (view.job.num ? "#" + view.job.num : "job " + view.job.id) : "";
    stagedFeedback.textContent = view.notice
      ? view.notice.label + (jobTag ? " · " + jobTag : "") + (view.notice.message ? " · " + view.notice.message : "")
      : "";
  };
  const countTotals = (s) => {
    const allCounts = s.n && typeof s.n === "object" ? s.n : {};
    const counts = selectedRepo ? safeArray(s.byRepo).filter((row) => row.repo === selectedRepo)
      .reduce((total, row) => { total[row.st] = Number(row.n) || 0; return total; }, {}) : allCounts;
    const active = Object.entries(counts)
      .filter(([key]) => key !== "done" && key !== "drop")
      .reduce((total, [, amount]) => total + Math.max(0, Number(amount) || 0), 0);
    return {active, done:Math.max(0, Number(counts.done) || 0)};
  };
  const hourLabel = (ago) => { const d = new Date(Date.now() - ago * 3600000); return String(d.getHours()).padStart(2, "0") + "h"; };
  /** Draw the backend's UTC merge counts over a rolling 30-day window. */
  const renderMerges = (s) => {
    const now = Date.now();
    const todayUtc = new Date(now).toISOString().slice(0, 10);
    const signature = JSON.stringify([s.mg, todayUtc]);
    if (signature === lastMergeSignature) return;
    lastMergeSignature = signature;
    const counts = new Map(safeArray(s.mg).map((item) => [item.d, Number(item.n) || 0]));
    const days = [];
    for (let i = 29; i >= 0; i--) {
      const key = new Date(now - i * 86400000).toISOString().slice(0, 10);
      days.push({date:key, count:counts.get(key) || 0});
    }
    const max = Math.max(1, ...days.map((day) => day.count));
    byId("aoMergeChart").innerHTML = '<div class="ao-merge-graph" role="img" aria-label="Gráfico de integrações dos últimos 30 dias">' +
      days.map((day) => '<span title="' + day.date + ': ' + day.count + ' PRs" style="height:' + Math.max(2, day.count / max * 100) + '%" class="' + (day.count ? "ao-bar-live" : "ao-bar-zero") + '"></span>').join("") +
      '</div><div class="ao-trend-labels"><span>Há 30 dias</span><span>Hoje (UTC)</span></div>';
  };
  /**
   * Draw 24 hourly buckets of really-completed tasks (s.prod24 = {g,n} by hours-ago).
   * @param {object} s Server state with prod24 productivity history.
   */
  const renderProductivity = (s) => {
    const rows = safeArray(s.prod24);
    const signature = JSON.stringify([rows, Math.floor(Date.now() / 3600000)]);
    if (signature === lastProdSignature) return;
    lastProdSignature = signature;
    const total = rows.reduce((t, r) => t + (Number(r.n) || 0), 0);
    const counts = new Map(rows.map((r) => [Number(r.g), Number(r.n) || 0]));
    const hours = [];
    for (let i = 23; i >= 0; i--) hours.push({ago: i, n: counts.get(i) || 0});
    const max = Math.max(1, ...hours.map((h) => h.n));
    const top = max <= 4 ? 5 : Math.ceil(max / 5) * 5;
    const yTicks = total
      ? [top, Math.round(top * 0.75), Math.round(top * 0.5), Math.round(top * 0.25), 0]
      : ["", "", "", "", 0];
    const xIdx = [0, 4, 8, 12, 16, 20, 23];
    const xLabels = xIdx.map((i) => hours[i].ago === 0 ? "Agora" : hourLabel(hours[i].ago));
    byId("aoProductivityChart").innerHTML =
      '<div class="ao-prod">' +
        '<div class="ao-prod-yaxis" aria-hidden="true">' + yTicks.map((t) => '<span>' + t + '</span>').join("") + '</div>' +
        '<div class="ao-prod-main">' +
        '<div class="ao-merge-graph" role="img" aria-label="' + (total
          ? "Tarefas concluídas por hora nas últimas 24 horas"
          : "Nenhuma tarefa concluída nas últimas 24 horas") + '">' +
            hours.map((h) => '<span title="' + hourLabel(h.ago) + ': ' + h.n + ' tarefas" style="height:' + (h.n ? Math.max(2, h.n / top * 100) : 0) + '%" class="' + (h.n ? "ao-bar-live" : "ao-bar-zero") + '"></span>').join("") +
          '</div>' +
          (total ? "" : '<span class="ao-prod-empty">Sem conclusões registadas</span>') +
          '<div class="ao-prod-xaxis" aria-hidden="true">' + xLabels.map((l) => '<span>' + l + '</span>').join("") + '</div>' +
        '</div>' +
      '</div>';
  };
  /**
   * Show measured host telemetry only while its heartbeat and state are fresh.
   * Older readings are labeled as history, never as current performance.
   * @param {object} s Server state with hosts, telemetry and connectivity.
   */
  const renderMachines = (s) => {
    const signature = JSON.stringify([s.hs, s.hst, s.o?.net, app.classList.contains("ao-data-stale"), Math.floor(Date.now() / 60000)]);
    if (signature === lastMachineSignature) return;
    lastMachineSignature = signature;
    const hosts = safeArray(s.hs), live = s.hst || {};
    const machineList = byId("aoMachines");
    if (!hosts.length) {
      machineList.innerHTML = '<p class="ao-empty">Ainda não existem máquinas configuradas.</p>';
      return;
    }
    machineList.innerHTML = hosts.slice(0, 4).map((host) => {
      const info = live[host.id] || {};
      const enabled = !!host.on_;
      const unavailable = enabled && s.o?.net?.[host.id] === false;
      const age = Date.now() - Number(info.t || 0);
      const recent = enabled && !unavailable && !app.classList.contains("ao-data-stale") && age >= 0 && age < 120_000;
      const status = !enabled ? "Desativada" : unavailable ? "Indisponível" : recent ? "Dados recentes" : "Por confirmar";
      const pieces = [];
      if (Number.isFinite(Number(info.cpu)) && Number(info.cpu) > 0) pieces.push(info.cpu + " núcleos");
      if (info.load != null && Number.isFinite(Number(info.load)) && Number(info.load) >= 0) {
        pieces.push("Carga " + Number(info.load).toLocaleString("pt-PT", {maximumFractionDigits:2}));
      }
      if (info.temp != null && Number.isFinite(Number(info.temp))) {
        pieces.push(Number(info.temp).toLocaleString("pt-PT", {maximumFractionDigits:1}) + " °C");
      }
      if (Array.isArray(info.mem) && Number(info.mem[0]) > 0) {
        const used = (Number(info.mem[0]) - Number(info.mem[1])) / 1024;
        pieces.push(used.toFixed(1) + " / " + (Number(info.mem[0]) / 1024).toFixed(1) + " GB RAM");
      }
      if (Array.isArray(info.disk) && Number(info.disk[0]) > 0) pieces.push(info.disk[1] + " GB livres");
      const lastSeen = Number(info.t);
      const timestamp = Number.isFinite(lastSeen) && lastSeen > 0
        ? new Date(lastSeen).toLocaleString("pt-PT", {day:"2-digit", month:"2-digit", hour:"2-digit", minute:"2-digit"})
        : null;
      const description = recent ? (pieces.join(" · ") || "Sem métricas de hardware")
        : timestamp ? "Última leitura: " + timestamp : "Sem leituras recentes";
      return '<div class="ao-machine-row"><span class="ao-host-dot ' + (recent ? "ao-host-up" : "ao-host-down") + '"></span><span><strong>' +
        escapeHTML(host.id) + '</strong><small>' + escapeHTML(description) + '</small></span><em>' + status + '</em></div>';
    }).join("");
  };
  const markStale = () => {
    app.classList.add("ao-data-stale");
    clearReactions();
    // Preserve the last task records, but never claim a worker is still active.
    for (const role of officeRoles) {
      const desk = stationNodes[role];
      if (!desk) continue;
      desk.dataset.mode = "unknown";
      desk.querySelector(".ao-station-state").textContent = "Por confirmar";
      desk.setAttribute("aria-label", "Ver agente " + names[role] + ". Estado por confirmar");
    }
    byId("aoMetricWorkers").textContent = "—";
    for (const role of roles) {
      const row = agentList.querySelector('[data-role="' + role + '"]');
      const status = row?.querySelector(".ao-agent-status");
      if (row) { row.dataset.state = "unknown"; row.classList.remove("ao-state-work", "ao-state-paused"); }
      if (status) status.textContent = "Por confirmar";
    }
    byId("aoAgentSummary").textContent = "Sem atualização recente";
    byId("aoMetricWorkers").textContent = "—";
    if (snapshot) renderMachines(snapshot);
  };
  /**
   * Reconcile the modern dashboard with a new authoritative state snapshot.
   * Unknown or invalid snapshots do not overwrite the last valid UI.
   * @param {object} s /api/state payload.
   */
  const renderDiagnostics = (s) => {
    const blockedHunts = safeArray(s.ops?.blocked_hunts);
    const target = byId("aoDiag");
    const rows = blockedHunts.map((entry) => {
      const health = entry.health || {};
      const repo = String(entry.repo || "Repositório desconhecido");
      const failures = Number(health.failures) || 0;
      const stage = health.stage ? " em " + health.stage : "";
      const code = health.exit_code == null ? "" : " · saída " + health.exit_code;
      const until = Number(health.until) > Date.now() ? " · suspensão até " + datum(health.until) : "";
      return '<div class="ao-diag-item ao-diag-warn"><span class="ao-diag-dot" aria-hidden="true"></span><span>Hunter suspenso em ' +
        escapeHTML(repo) + ' após ' + failures + ' falhas de preparação' + escapeHTML(stage + code + until) + '</span></div>';
    });
    target.innerHTML = rows.join("");
    target.hidden = rows.length === 0;
  };
  const render = (s) => {
    if (!s || typeof s !== "object" || s.error) return;
    snapshot = s;
    lastStateAt = Date.now();
    const serverTime = typeof s.t === "number" && Number.isFinite(s.t) ? s.t : null;
    const timeLabel = serverTime ? "Atualizado às " + datum(serverTime) : "Atualização recebida";
    const timeElement = byId("aoUpdatedAt");
    if (timeElement.textContent !== timeLabel) timeElement.textContent = timeLabel;
    app.classList.remove("ao-data-stale");
    const me = typeof s.me === "string" ? s.me.trim() : "";
    const greeting = greetWord() + (me ? ", " + me + " 👋" : " 👋");
    if (byId("aoGreeting").textContent !== greeting) byId("aoGreeting").textContent = greeting;
    const userName = me || "Utilizador";
    const nameEl = byId("aoUserName");
    if (nameEl && nameEl.textContent !== userName) nameEl.textContent = userName;
    const initials = me ? me.split(/\s+/).filter(Boolean).map((p) => p[0]).join("").slice(0, 2).toUpperCase() : "·";
    const initEl = byId("aoUserInitials");
    if (initEl && initEl.textContent !== initials) initEl.textContent = initials;
    const isPaused = paused(s), isOffline = offline(s), isWarn = !!s.o?.warn;
    const state = byId("aoOfficeStatus");
    state.className = "ao-status-chip" + (isPaused || isWarn || isOffline ? " is-paused" : "");
    const statusLabel = isOffline ? "Serviços externos indisponíveis" : isPaused ? "Escritório em pausa" : isWarn ? "Aviso de utilização" : "Sistema operacional";
    if (state.textContent !== statusLabel) state.textContent = statusLabel;
    const approvalsPending = safeArray(s.q).filter((j) => j.st === "approve" && (!selectedRepo || j.repo === selectedRepo)).length;
    const bellDot = byId("aoBell")?.querySelector(".ao-bell-dot");
    if (bellDot) bellDot.hidden = approvalsPending === 0;
    byId("aoBell")?.setAttribute("aria-label", approvalsPending ? approvalsPending + " decisão(ões) pendente(s)" : "Sem notificações");
    const connection = isOffline ? "Problema em serviços externos" : "Dados em tempo real";
    if (byId("aoConnection").textContent !== connection) byId("aoConnection").textContent = connection;
    byId("aoConnection").closest(".ao-sidebar-footer").dataset.state = isOffline ? "offline" : isPaused ? "paused" : isWarn ? "warning" : "live";
    const officeMode = isPaused ? "Escritório em pausa" : isOffline ? "Dados externos indisponíveis" : "Estado sincronizado";
    byId("aoOfficeMode").textContent = officeMode;
    byId("aoOfficeMode").classList.toggle("is-paused", isPaused || isOffline || isWarn);
    const pauseLabel = isPaused ? "Retomar escritório" : "Pausar escritório";
    const pauseButton = byId("aoPause");
    byId("aoPauseLabel").textContent = pauseLabel;
    pauseButton.setAttribute("aria-label", pauseLabel);
    pauseButton.title = pauseLabel;
    pauseButton.dataset.state = isPaused ? "paused" : "running";
    pauseButton.disabled = !s.o;
    renderDiagnostics(s); updateAgents(s); renderPipeline(s); renderOfficeCheck(s); renderStagedPipeline(s); renderRepos(s); renderLists(s); renderMerges(s); renderProductivity(s); renderMachines(s);
    if (selected) showAgent(selected, false);
  };
  const overlay = byId("aoAgentOverlay");
  /**
   * Open or refresh an accessible read-only detail view for one agent.
   * @param {string} role Authorized agent key.
   * @param {boolean} focus Whether to move keyboard focus to the dialog.
   */
  const showAgent = (role, focus = true) => {
    if (!snapshot || !roles.includes(role)) return;
    const w = snapshot.w?.[role] || {};
    const stale = app.classList.contains("ao-data-stale");
    const stage = stale ? "unknown" : mode(snapshot, role);
    selected = role;
    byId("aoDetailTitle").textContent = names[role];
    const avatar = byId("aoDetailAvatar");
    if (avatar) { avatar.src = "/assets/agents/" + role + ".webp"; avatar.alt = names[role]; }
    const jobs = safeArray(snapshot.q);
    const job = w.job != null ? jobs.find((j) => String(j.id) === String(w.job)) : null;
    const recent = lastCommands(w);
    const current = typeof w.say === "string" ? w.say.trim() : "";
    const signature = JSON.stringify([role, stage, stale, w.md, w.say, w.mood, recent, job?.id, job?.title]);
    if (!focus && signature === lastDetailSignature) return;
    lastDetailSignature = signature;
    const rows = [
      ["Estado",stale ? "Por confirmar — dados desatualizados" : statusText[stage]],
      ["Modelo",w.md ? A.mdn(w.md) : "Sem modelo atribuído"],
      ["Atividade",current ? (stage === "work" ? current : "Último registo: " + current) : "Sem informação adicional"]
    ];
    if (!stale && stage !== "work" && w.idle_reason) {
      rows.push(["Motivo da espera", String(w.idle_reason)]);
    }
    if (Object.hasOwn(resultText, w.mood)) rows.push(["Resultado recente", resultText[w.mood].replace("Último resultado: ", "")]);
    if (job) {
      rows.push(["Tarefa",job.title]);
      rows.push(["Tipo de trabalho",A.workLabel ? A.workLabel(job.work_class) : job.kind]);
    }
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
  const listOverlay = byId("aoListOverlay");
  let listReturnFocus = null;
  const listViews = {
    tasks: {title: "Tarefas e decisões", icon: "tasks.svg", count: "aoTaskSummary"},
    prs: {title: "Pull Requests", icon: "pulls.svg", count: "aoPRSummary"},
    activity: {title: "Histórico de atividade", icon: "activity-wave.svg", count: "aoActivitySummary"}
  };
  const hideList = () => {
    if (listOverlay.hidden) return;
    listOverlay.hidden = true;
    if (listReturnFocus?.isConnected) listReturnFocus.focus();
  };
  const showList = (kind) => {
    const view = listViews[kind];
    if (!view) return;
    listReturnFocus = document.activeElement;
    byId("aoListDialogTitle").textContent = view.title;
    byId("aoListDialogIcon").src = "/assets/icons/" + view.icon;
    byId("aoListDialogCount").textContent = byId(view.count).textContent;
    const content = byId("aoListDialogContent");
    content.innerHTML = '<div class="ao-list ao-list-expanded">' + (listDialogData[kind] || '<p class="ao-empty">Sem registos.</p>') + '</div>';
    if (kind === "tasks" && !stagedPanel.hidden) {
      const stagedCopy = stagedPanel.cloneNode(true);
      stagedCopy.removeAttribute("id");
      stagedCopy.classList.remove("ao-staged-panel");
      stagedCopy.setAttribute("aria-label", "Estado do pipeline test-first");
      stagedCopy.querySelectorAll("[id]").forEach((node) => node.removeAttribute("id"));
      content.appendChild(stagedCopy);
    }
    listOverlay.hidden = false;
    byId("aoListClose").focus();
  };
  byId("aoListClose").addEventListener("click", hideList);
  listOverlay.addEventListener("click", (e) => { if (e.target === listOverlay) hideList(); });
  const onListModalKeydown = (e) => {
    if (listOverlay.hidden) return;
    if (e.key === "Escape") { e.preventDefault(); hideList(); return; }
    if (e.key !== "Tab") return;
    const items = Array.from(listOverlay.querySelectorAll("button:not([hidden]):not(:disabled)"));
    if (!items.length) { e.preventDefault(); return; }
    const first = items[0], last = items[items.length - 1];
    if (e.shiftKey && (document.activeElement === first || document.activeElement === listOverlay.querySelector(".ao-list-dialog"))) {
      e.preventDefault(); last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault(); first.focus();
    }
  };
  document.addEventListener("keydown", onListModalKeydown);
  byId("aoDetailClose").addEventListener("click", hideAgent);
  overlay.addEventListener("click", (e) => { if (e.target === overlay) hideAgent(); });
  const onModalKeydown = (e) => {
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
  };
  document.addEventListener("keydown", onModalKeydown);
  const handlers = {repos:"bRepos",hosts:"bHosts",costs:"bCost",settings:"bSet",steward:"bSteward"};
  app.addEventListener("click", (event) => {
    const node = event.target.closest("[data-role],[data-agent],[data-job],[data-pr-url],[data-target],[data-open],[data-open-list]");
    if (!node) return;
    if (node.dataset.openList) { showList(node.dataset.openList); return; }
    if (node.dataset.role || node.dataset.agent) { showAgent(node.dataset.role || node.dataset.agent); return; }
    if (node.dataset.job) { A.job?.(Number(node.dataset.job)); return; }
    if (node.dataset.prUrl) { A.open?.(node.dataset.prUrl); return; }
    if (node.dataset.open) { byId(handlers[node.dataset.open])?.click(); return; }
    if (node.dataset.target) {
      const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches;
      byId(node.dataset.target)?.scrollIntoView({
        behavior: animationsOn && !reduced ? "smooth" : "auto", block: "start"
      });
      // Only sidebar items drive the active-nav highlight; "Ver todas" links don't.
      if (node.classList.contains("ao-nav-item")) {
        app.querySelectorAll(".ao-nav-item").forEach((n) => n.classList.toggle("is-active", n === node));
      }
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
    A.stopOfficeMotion?.();
    document.removeEventListener("keydown", onModalKeydown);
    document.removeEventListener("keydown", onListModalKeydown);
    document.removeEventListener("visibilitychange", onVisibilityChange);
    if (staleTimer !== null) clearInterval(staleTimer);
    console.error("Apollo Control Room: reverting to original interface", error);
    if (pixelStage) {
      if (pixelCanvas && pixelCanvas.parentElement !== pixelStage) pixelStage.appendChild(pixelCanvas);
      if (pixelMerge && pixelMerge.parentElement !== pixelStage) pixelStage.appendChild(pixelMerge);
    }
    if (pixelTip && pixelTipParent) {
      const next = pixelTipNext?.parentElement === pixelTipParent ? pixelTipNext : null;
      pixelTipParent.insertBefore(pixelTip, next);
    }
    app.remove();
    legacy.replaceWith(...Array.from(legacy.children));
    document.body.classList.remove("ao-enabled");
    document.body.classList.add("ao-fallback-no-pixel");
    A.disablePixelArt = true;
    A.pixelDashboard = false;
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
  // When startup recovery already restored the old interface, do not
  // register a timer or any additional work for the removed dashboard.
  if (!healthy) return; // Keep the connection status timers scoped to the mounted dashboard.
  // Live local clock (presentation of the real current time, not server data).
  const months = ["jan","fev","mar","abr","mai","jun","jul","ago","set","out","nov","dez"];
  const tickClock = () => {
    if (!app.isConnected) { clearInterval(clockTimer); return; }
    const d = new Date();
    const hm = String(d.getHours()).padStart(2, "0") + ":" + String(d.getMinutes()).padStart(2, "0");
    const dt = d.getDate() + " " + months[d.getMonth()] + " " + d.getFullYear();
    const c = byId("aoClock"), dl = byId("aoDate");
    if (c && c.textContent !== hm) c.textContent = hm;
    if (dl && dl.textContent !== dt) dl.textContent = dt;
  };
  tickClock();
  const clockTimer = setInterval(tickClock, 15000);
  // Keep the connection label honest if WebSocket and polling stop updating.
  staleTimer = setInterval(() => {
    if (!healthy || !app.isConnected) { clearInterval(staleTimer); return; }
    if (Date.now() - (lastStateAt || startedAt) <= 12000) return;
    const state = byId("aoOfficeStatus");
    if (!app.classList.contains("ao-data-stale")) markStale();
    state.className = "ao-status-chip is-paused";
    if (state.textContent !== "Dados desatualizados") state.textContent = "Dados desatualizados";
    byId("aoConnection").textContent = "Sem atualizações recentes";
    byId("aoUpdatedAt").textContent = "Sem atualização recente";
    byId("aoConnection").closest(".ao-sidebar-footer").dataset.state = "stale";
  }, 5000);
})();
