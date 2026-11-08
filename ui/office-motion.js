/* Apollo Office — real pipeline handoffs as lightweight character walks.
 * Display-only: each walk represents an observed stage transition for the SAME job.
 * No calls to providers, job mutations or extra state requests.
 */
(() => {
  "use strict";
  const A = window.AO;
  const app = document.getElementById("aoControlRoom");
  const stage = document.getElementById("aoStations");
  const office = document.querySelector(".ao-office");
  if (!A?.modernRoom || !app || !stage || !office) return;
  // A second script evaluation must not leave two polling subscribers,
  // duplicate live-region IDs or independent animations in the same room.
  if (typeof A.stopOfficeMotion === "function") A.stopOfficeMotion();

  // Map only stages the server can tie to a specific worker. Other states do
  // not imply a handoff and therefore never make a character travel.
  const owners = Object.freeze({
    vet: "vet", hunt: "hunter", repro: "repro", fix: "fixer",
    ci: "ci", review: "reviewer", gate: "submit", submit: "submit"
  });
  const names = A.NM || {};
  let previous = null;
  let previousAt = 0;
  let active = null;
  const reduceMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)");
  const motionControl = document.getElementById("aoMotion");

  // This label belongs to the scene and is not an action button.
  const indicator = document.createElement("div");
  indicator.id = "aoTransfer";
  indicator.className = "ao-transfer-label";
  indicator.hidden = true;
  indicator.setAttribute("role", "status");
  indicator.setAttribute("aria-live", "polite");
  office.appendChild(indicator);

  /**
   * Check that an observed handoff is eligible for visible animation.
   * Disabled motion, reduced-motion preferences and stale data prohibit it.
   * @param {object} state Latest server snapshot.
   * @returns {boolean}
   */
  const stateAllowed = (state) => {
    if (!A.modernRoom || !app.isConnected || document.visibilityState === "hidden") return false;
    if (!motionControl?.checked || reduceMotion?.matches) return false;
    if (app.classList.contains("ao-data-stale")) return false;
    return !state?.o?.man && !state?.o?.pause && !state?.o?.off;
  };

  /**
   * Cancel an active walker and restore both involved desk characters.
   * The cleanup is idempotent so animation and unmount races remain safe.
   */
  const cancel = () => {
    if (!active) return;
    const { animation, finish } = active;
    active = null;
    try { animation.cancel(); } catch {}
    finish();
  };

  /**
   * Animate the visual handoff of an observed job between two agent desks.
   * Does not submit jobs, call models or imply physical agent communication.
   * @param {{id:string,title:string,from:string,to:string}} transition
   */
  const play = ({ id, title, from, to }) => {
    const src = stage.querySelector('[data-role="' + from + '"]');
    const dst = stage.querySelector('[data-role="' + to + '"]');
    if (!src || !dst || active || !stage.isConnected) return;

    const sprite = src.querySelector(".ao-person");
    const roomBox = stage.getBoundingClientRect();
    const a = src.getBoundingClientRect();
    const b = dst.getBoundingClientRect();
    if (!sprite || !roomBox.width || !roomBox.height || !a.width || !b.width) return;

    const fromX = a.left - roomBox.left + a.width * .24;
    const fromY = a.top - roomBox.top + a.height * .14;
    const toX = b.left - roomBox.left + b.width * .24;
    const toY = b.top - roomBox.top + b.height * .14;
    const visitor = document.createElement("div");
    visitor.className = "ao-walking-agent";
    visitor.style.left = fromX + "px";
    visitor.style.top = fromY + "px";
    visitor.setAttribute("aria-hidden", "true");
    const copy = sprite.cloneNode(true);
    // Gradient IDs in cloned SVGs must remain unique.
    const suffix = "-walk-" + String(id).replace(/[^0-9]/g, "");
    for (const item of copy.querySelectorAll("[id]")) {
      const oldId = item.id;
      const newId = oldId + suffix;
      item.id = newId;
      for (const el of copy.querySelectorAll("*")) {
        for (const att of ["fill", "stroke", "filter", "clip-path", "mask"]) {
          const val = el.getAttribute(att);
          if (val === "url(#" + oldId + ")") el.setAttribute(att, "url(#" + newId + ")");
        }
      }
    }
    visitor.appendChild(copy);
    stage.appendChild(visitor);
    src.classList.add("ao-travelling");
    dst.classList.add("ao-receiving");
    dst.classList.add("ao-v2-interacting");

    indicator.textContent =
      (names[from] || from) + " → " + (names[to] || to) + " · tarefa #" + id +
      (title ? " · " + title : "");
    indicator.hidden = false;

    // The handoff is a visual metaphor: travel to the next station and back.
    // No claim is made that an actual agent has physically moved or communicated.
    const dx = toX - fromX, dy = toY - fromY;
    const direction = dx < 0 ? -1 : 1;
    // Travel via the central aisle so walkers do not pass through workstations.
    const corridorY = Math.max(18, Math.min(roomBox.height - 45, roomBox.height * .56)) - fromY;
    // On narrow screens the desks form five rows: use the vertical central
    // aisle instead of walking through a horizontal row of workstations.
    const narrow = window.matchMedia?.("(max-width: 530px)")?.matches;
    const corridorX = Math.max(12, Math.min(roomBox.width - 30, roomBox.width * .5)) - fromX;
    // Two mobile desks may share a row: dy is then zero, so choose a
    // gutter outside both desks before walking horizontally.
    const sameMobileRow = narrow && Math.abs(dy) < Math.min(a.height, b.height) * .25;
    const rowTop = Math.min(a.top, b.top) - roomBox.top;
    const rowBottom = Math.max(a.top + a.height, b.top + b.height) - roomBox.top;
    const gutterY = (roomBox.height - rowBottom > 14
      ? Math.min(roomBox.height - 10, rowBottom + 10)
      : Math.max(10, rowTop - 10)) - fromY;
    let finished = false;
    const finish = () => {
      if (finished) return;
      finished = true;
      visitor.remove();
      src.classList.remove("ao-travelling");
      dst.classList.remove("ao-receiving");
      dst.classList.remove("ao-v2-interacting");
      indicator.hidden = true;
      if (active?.visitor === visitor) active = null;
    };
    if (typeof visitor.animate !== "function") { finish(); return; }
    let animation;
    try {
      const route = sameMobileRow
        ? [[0, 0], [0, gutterY], [corridorX, gutterY], [dx, gutterY],
           [dx, dy], [dx, dy], [dx, gutterY], [0, gutterY], [0, 0]]
        : narrow
          ? [[0, 0], [corridorX, 0], [corridorX, dy * .5], [corridorX, dy],
             [dx, dy], [dx, dy], [corridorX, dy], [corridorX, 0], [0, 0]]
          : [[0, 0], [0, corridorY], [dx * .55, corridorY], [dx, corridorY],
             [dx, dy], [dx, dy], [dx, corridorY], [0, corridorY], [0, 0]];
      const offsets = [0, .16, .33, .44, .54, .65, .75, .88, 1];
      const frames = route.map(([x, y], i) => ({
        transform: "translate(" + x + "px," + y + "px) scaleX(" + (i < 5 ? direction : -direction) + ")",
        offset: offsets[i]
      }));
      animation = visitor.animate(frames, { duration: 3900, easing: "ease-in-out", fill: "forwards" });
    } catch (error) {
      // A failed animation must never leave the agent invisible.
      finish();
      console.warn("Apollo Office: handoff animation unavailable", error);
      return;
    }
    active = { animation, finish, visitor };
    animation.onfinish = finish;
    animation.oncancel = finish;
  };

  if (Array.isArray(A.S?.q)) {
    previous = new Map(A.S.q.filter(j => j && j.id != null && owners[j.st])
      .map(j => [String(j.id), { stage: j.st, title: String(j.title || "") }]));
    previousAt = Date.now();
  }
  const unsubscribe = A.subscribeState((s) => {
    if (!app.isConnected || !A.modernRoom) { dispose(); return; }
    if (!s || !Array.isArray(s.q)) return;
    const current = new Map(
      s.q.filter(j => j && j.id != null && owners[j.st])
        .map(j => [String(j.id), { stage: j.st, title: String(j.title || "") }])
    );
    // The animation must represent a recently observed transition, not a
    // historical change discovered after a long disconnect or suspended tab.
    const now = Date.now();
    const contiguous = previousAt && now - previousAt <= 10000;
    if (previous && contiguous && stateAllowed(s) && !active) {
      for (const [id, next] of current) {
        const prior = previous.get(id);
        if (!prior || prior.stage === next.stage) continue;
        const from = owners[prior.stage], to = owners[next.stage];
        if (!from || !to || from === to) continue;
        play({id, title: next.title, from, to});
        break; // One transition at a time prevents an animation pile-up.
      }
    }
    previous = current;
    previousAt = now;
    if (active && !stateAllowed(s)) cancel();
  });

  const onMotionChange = () => { if (!motionControl.checked) cancel(); };
  /**
   * Cancela deslocações e poses se o sistema pedir movimento reduzido.
   * Mantém os estados reais dos agentes e os respetivos dados intactos.
   */
  const onReducedMotion = () => {
    if (reduceMotion.matches) {
      cancel();
      window.AOOfficeV2?.clearTransitions(stage);
    }
  };
  const onVisibilityChange = () => { if (document.hidden) cancel(); };
  /**
   * Tear down animation observers and restore the original dashboard cleanly.
   */
  const dispose = () => {
    cancel();
    unsubscribe();
    motionControl?.removeEventListener?.("change", onMotionChange);
    reduceMotion?.removeEventListener?.("change", onReducedMotion);
    document.removeEventListener?.("visibilitychange", onVisibilityChange);
    indicator.remove();
    if (A.cancelOfficeMotion === cancel) delete A.cancelOfficeMotion;
    if (A.stopOfficeMotion === dispose) delete A.stopOfficeMotion;
  };
  motionControl?.addEventListener("change", onMotionChange);
  reduceMotion?.addEventListener?.("change", onReducedMotion);
  document.addEventListener("visibilitychange", onVisibilityChange);
  // Stale-data recovery must stop movement without removing the observer:
  // the next confirmed state can then restart the animation controller.
  A.cancelOfficeMotion = cancel;
  A.stopOfficeMotion = dispose;
})();
