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

  // Map only stages the server can tie to a specific worker. Other states do
  // not imply a handoff and therefore never make a character travel.
  const owners = Object.freeze({
    vet: "vet", hunt: "hunter", repro: "repro", fix: "fixer",
    ci: "ci", review: "reviewer", gate: "submit", submit: "submit"
  });
  const names = A.NM || {};
  let previous = null;
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

  const stateAllowed = (state) => {
    if (!A.modernRoom || !app.isConnected || document.visibilityState === "hidden") return false;
    if (!motionControl?.checked || reduceMotion?.matches) return false;
    if (app.classList.contains("ao-data-stale")) return false;
    return !state?.o?.man && !state?.o?.pause && !state?.o?.off;
  };

  const cancel = () => {
    if (!active) return;
    const { animation, finish } = active;
    active = null;
    try { animation.cancel(); } catch {}
    finish();
  };

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

    indicator.textContent =
      (names[from] || from) + " → " + (names[to] || to) + " · tarefa #" + id +
      (title ? " · " + title : "");
    indicator.hidden = false;

    // The handoff is a visual metaphor: travel to the next station and back.
    // No claim is made that an actual agent has physically moved or communicated.
    const dx = toX - fromX, dy = toY - fromY;
    const direction = dx < 0 ? -1 : 1;
    let finished = false;
    const finish = () => {
      if (finished) return;
      finished = true;
      visitor.remove();
      src.classList.remove("ao-travelling");
      dst.classList.remove("ao-receiving");
      indicator.hidden = true;
      if (active?.visitor === visitor) active = null;
    };
    if (typeof visitor.animate !== "function") { finish(); return; }
    let animation;
    try {
      animation = visitor.animate(
        [
          { transform: "translate(0px, 0px) scaleX(" + direction + ")", offset: 0 },
          { transform: "translate(" + dx * .48 + "px," + (dy * .48 - 9) + "px) scaleX(" + direction + ")", offset: .32 },
          { transform: "translate(" + dx + "px," + dy + "px) scaleX(" + direction + ")", offset: .56 },
          { transform: "translate(" + dx + "px," + dy + "px) scaleX(" + -direction + ")", offset: .7 },
          { transform: "translate(0px, 0px) scaleX(" + -direction + ")", offset: 1 }
        ],
        { duration: 2800, easing: "ease-in-out", fill: "forwards" }
      );
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
  }
  const unsubscribe = A.subscribeState((s) => {
    if (!app.isConnected || !A.modernRoom) { cancel(); unsubscribe(); return; }
    if (!s || !Array.isArray(s.q)) return;
    const current = new Map(
      s.q.filter(j => j && j.id != null && owners[j.st])
        .map(j => [String(j.id), { stage: j.st, title: String(j.title || "") }])
    );
    if (previous && stateAllowed(s) && !active) {
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
    if (active && !stateAllowed(s)) cancel();
  });

  motionControl?.addEventListener("change", () => { if (!motionControl.checked) cancel(); });
  reduceMotion?.addEventListener?.("change", () => { if (reduceMotion.matches) cancel(); });
  document.addEventListener("visibilitychange", () => { if (document.hidden) cancel(); });
})();
