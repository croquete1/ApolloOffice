/* Apollo Office – premium 2.5D room and illustrated staff.
 * Visual-only. Never starts work or infers worker status from the scene. */
(() => {
  "use strict";
  const roles = ["scout","guard","vet","watcher","hunter","repro","fixer","ci","reviewer","submit"];
  const profiles = [
    ["#bd8367","#242c3d","#60a9ae","#c58a64","short"],
    ["#ecc49d","#292c3a","#4f7d91","#aa7968","side"],
    ["#8c5d47","#171f2b","#b99ada","#b0756b","curly"],
    ["#dcab87","#6a463d","#c48f59","#d2a77d","part"],
    ["#a87559","#242b37","#8a9fc3","#a77056","messy"],
    ["#efd0b5","#65403c","#cd806c","#cb9070","bob"],
    ["#c99876","#252a36","#56a9bb","#c08a68","crop"],
    ["#ad755b","#423744","#9a8fce","#966a53","waves"],
    ["#e3b38b","#272a34","#7abf9b","#c99f7b","swept"],
    ["#b9876d","#53443e","#8ca6bf","#d0a789","side"]
  ];
  const symbols = {
    scout:'<path d="M5 15l5-5 4 3 6-7"/><circle cx="5" cy="15" r="2"/><circle cx="20" cy="6" r="2"/>',
    guard:'<path d="M12 2l9 4v6c0 6-4 9-9 11-5-2-9-5-9-11V6z"/><path d="M8 12l3 3 5-6"/>',
    vet:'<rect x="5" y="4" width="14" height="17" rx="2"/><path d="M9 3h6v4H9zM8 12l2 2 3-3M8 17h8"/>',
    watcher:'<path d="M3 18l5-6 4 3 4-9 5 5"/><path d="M3 22h19M3 3v19"/>',
    hunter:'<circle cx="10" cy="10" r="6"/><path d="M14.5 14.5L21 21"/>',
    repro:'<path d="M8 9h8v10H8zM7 7l-3-3m13 3 3-3M4 12h4m8 0h4M4 18l4-3m8 0 4 3M9 6h6v3"/>',
    fixer:'<path d="M9 6L3 12l6 6M15 6l6 6-6 6M14 3l-4 18"/>',
    ci:'<path d="M12 2l2 3 4-1 1 4 4 2-2 4 1 4-4 1-3 4-4-2-4 1-1-4-4-2 2-4-1-4 4-1z"/><circle cx="12" cy="12" r="3"/>',
    reviewer:'<path d="M4 4h16v16H4zM7 9h10M7 13h6M7 17l2 2 4-5"/>',
    submit:'<path d="M12 17V3m-5 5 5-5 5 5M4 15v5h16v-5"/>'
  };
  const hair = {
    short:"M49 51Q43 18 68 17Q93 13 103 40L97 50Q92 30 81 29Q58 34 49 51Z",
    side:"M50 54Q39 24 65 18Q97 13 103 40Q95 47 87 34Q72 25 53 47L49 58Z",
    curly:"M46 51Q35 35 47 28Q44 13 60 18Q70 8 82 19Q100 15 104 32Q115 40 100 52L94 38Q73 24 52 44Z",
    part:"M47 53Q39 24 69 18Q98 15 103 47L96 46Q89 31 78 28Q68 42 50 44Z",
    messy:"M47 53L43 31L54 23L56 12L68 24L77 14L85 23L98 19L105 37L96 51Q86 33 68 31Q53 35 47 53Z",
    bob:"M47 43Q44 18 74 18Q102 18 102 43L107 76L96 78L93 41Q73 33 54 43L51 76L41 76Z",
    crop:"M47 48Q41 22 66 20Q94 16 102 47Q81 31 63 35L48 51Z",
    waves:"M45 52Q40 19 70 18Q96 17 104 41L102 75L95 80L92 44Q82 29 59 38L50 74L42 76Z",
    swept:"M46 50Q40 23 60 20Q84 9 102 32L99 50Q95 32 83 32Q61 39 46 50Z"
  };
  const accessories = (i) => {
    let html = "";
    if (i === 2 || i === 7) html += '<path d="M50 55h18m5 0h17" stroke="#263446" stroke-width="2"/><rect x="49" y="50" width="19" height="13" rx="5" fill="none" stroke="#24384d" stroke-width="3"/><rect x="73" y="50" width="19" height="13" rx="5" fill="none" stroke="#24384d" stroke-width="3"/>';
    if (i === 1 || i === 4 || i === 8) html += '<path d="M47 55Q41 17 74 16Q103 15 104 56" fill="none" stroke="#233449" stroke-width="6"/><rect x="41" y="49" width="8" height="21" rx="4" fill="#3a5165"/><rect x="99" y="49" width="8" height="21" rx="4" fill="#3a5165"/>';
    if (i === 5) html += '<circle cx="92" cy="69" r="3" fill="#efccaa"/>';
    return html;
  };
  const character = (i) => {
    const role = roles[i], [skin,hairColor,jacket,trim,hairType] = profiles[i];
    const id = "ao-v2-" + role;
    return '<svg class="ao-person ao-v2-character" viewBox="0 0 150 188" focusable="false" aria-hidden="true" xmlns="http://www.w3.org/2000/svg">' +
      '<defs><linearGradient id="' + id + '-skin" x1="0" y1="0" x2="0.9" y2="1"><stop stop-color="' + skin + '"/><stop offset=".65" stop-color="' + skin + '"/><stop offset="1" stop-color="#704b42"/></linearGradient>' +
      '<linearGradient id="' + id + '-coat" x1="0" y1="0" x2="1" y2="1"><stop stop-color="' + jacket + '"/><stop offset="1" stop-color="#26374c"/></linearGradient></defs>' +
      '<ellipse cx="76" cy="181" rx="35" ry="5" fill="#08101b" opacity=".34"/>' +
      '<g class="ao-v2-legs"><g class="ao-v2-leg ao-v2-leg-left"><path d="M62 133L56 174L70 176L79 139" fill="#263348"/><path d="M55 169L70 171L74 181L49 181Q48 174 55 169Z" fill="#171f2c"/><path d="M52 179h22" stroke="#6a7789" stroke-width="2"/></g>' +
      '<g class="ao-v2-leg ao-v2-leg-right"><path d="M80 137L87 175L102 174L96 132" fill="#303d50"/><path d="M88 171L103 170L109 181L85 181Z" fill="#1b2231"/><path d="M87 179h22" stroke="#6a7789" stroke-width="2"/></g></g>' +
      '<g class="ao-v2-torso"><path d="M63 77L61 94L88 94L87 76Z" fill="url(#' + id + '-skin)"/>' +
      '<path d="M50 91Q59 82 71 85L77 94L84 85Q100 83 106 95L105 141Q77 151 47 142Z" fill="url(#' + id + '-coat)" stroke="#20344b" stroke-width="1.3"/>' +
      '<path d="M68 87L76 103L84 87" fill="#f1e6d7"/><path d="M74 101L77 134L81 101" fill="' + trim + '" opacity=".7"/>' +
      '<path d="M54 96L67 108M98 96L85 108" stroke="#d5dfe8" opacity=".21" stroke-width="2"/>' +
      '<path d="M67 106L70 125M85 105L82 125" stroke="#101b28" opacity=".35" stroke-width="2"/>' +
      '<g class="ao-arm ao-arm-left ao-v2-arm ao-v2-arm-left"><path d="M53 94Q36 103 42 123L55 131" stroke="' + jacket + '" stroke-width="14" stroke-linecap="round" fill="none"/><path d="M54 130q9 1 11 7q-5 8-13 3l-5-5Z" fill="url(#' + id + '-skin)"/></g>' +
      '<g class="ao-arm ao-arm-right ao-v2-arm ao-v2-arm-right"><path d="M100 94Q116 102 112 124L98 134" stroke="' + jacket + '" stroke-width="14" stroke-linecap="round" fill="none"/><path d="M97 133q-9 1-10 8q7 7 13 1l5-5Z" fill="url(#' + id + '-skin)"/></g>' +
      '<path d="M71 118h13v17H71z" fill="#dcecf0" opacity=".3"/><rect x="69" y="117" width="17" height="19" rx="3" fill="#ebf0f5" opacity=".88"/><path d="M72 123h11M72 128h7" stroke="' + jacket + '" stroke-width="1.5"/></g>' +
      '<g class="ao-head ao-v2-head"><ellipse cx="49" cy="57" rx="6" ry="10" fill="' + skin + '"/><ellipse cx="100" cy="57" rx="6" ry="10" fill="' + skin + '"/>' +
      '<ellipse cx="75" cy="54" rx="27" ry="32" fill="url(#' + id + '-skin)"/>' +
      '<path d="' + hair[hairType] + '" fill="' + hairColor + '"/><path d="M52 39Q75 19 96 37" stroke="#ffffff" stroke-opacity=".09" stroke-width="4" fill="none"/>' +
      '<g class="ao-eyes ao-v2-eyes"><path d="M58 55q6-3 11 0M81 55q6-3 11 0" stroke="#2a2c32" stroke-width="2.8" fill="none" stroke-linecap="round"/><circle cx="64" cy="56" r="2.4" fill="#1b2839"/><circle cx="86" cy="56" r="2.4" fill="#1b2839"/></g>' +
      '<path d="M76 59l-2 6h4" stroke="#875d51" stroke-width="1.2" fill="none" stroke-linecap="round"/><path d="M68 71Q75 75 82 70" stroke="#814c4b" stroke-width="1.7" fill="none" stroke-linecap="round"/>' +
      accessories(i) + '</g>' +
      '<g class="ao-v2-symbol" transform="translate(66 114) scale(.65)" stroke="#1c4350" fill="none" stroke-width="2.3" stroke-linecap="round" stroke-linejoin="round">' + symbols[role] + '</g>' +
      '</svg>';
  };
  const scene = [
    '<div class="ao-v2-architecture" aria-hidden="true">',
    '  <div class="ao-v2-backwall"></div>',
    '  <div class="ao-v2-window ao-v2-window-left"><div class="ao-v2-skyline"></div></div>',
    '  <div class="ao-v2-window ao-v2-window-right"><div class="ao-v2-skyline"></div></div>',
    '  <div class="ao-v2-panel-light ao-v2-panel-light-left"></div><div class="ao-v2-panel-light ao-v2-panel-light-right"></div>',
    '  <div class="ao-v2-shelf"><span></span><span></span><span></span></div>',
    '  <div class="ao-v2-plant ao-v2-plant-left"><i></i><i></i><i></i><i></i><b></b></div>',
    '  <div class="ao-v2-plant ao-v2-plant-right"><i></i><i></i><i></i><i></i><b></b></div>',
    '  <div class="ao-v2-rug"></div>',
    '  <div class="ao-v2-room-label"><span class="ao-v2-room-label-line"></span>APOLLO <em>OFFICE</em><small>ESTÚDIO DE DESENVOLVIMENTO</small></div>',
    '</div>'
  ].join("");
  function decorate(office, stations) {
    if (!office || !stations || office.dataset.visualVersion === "2") return;
    office.dataset.visualVersion = "2";
    office.insertAdjacentHTML("afterbegin", scene);
    const children = stations.querySelectorAll(".ao-station");
    children.forEach((node, i) => {
      node.insertAdjacentHTML("beforeend",
        '<span class="ao-v2-desk-legs" aria-hidden="true"></span>' +
        '<span class="ao-v2-mug" aria-hidden="true"></span>' +
        '<span class="ao-v2-notes" aria-hidden="true"></span>' +
        '<span class="ao-v2-worklight" aria-hidden="true"></span>');
      node.dataset.personality = i % 4 === 0 ? "focused" : i % 4 === 1 ? "observant" : i % 4 === 2 ? "analytical" : "calm";
      // A canceled arrival must not erase a newer rise (or vice versa).
      // If a replacement animation with the same name is running, let its
      // own completion clear the class rather than handling the old cancel.
      const stopPose = event => {
        const poseClass = event.animationName === "ao-v2-arrive" ? "ao-v2-arriving"
          : event.animationName === "ao-v2-rise" ? "ao-v2-rising" : null;
        if (!poseClass || (event.target && !event.target.classList?.contains("ao-v2-character"))) return;
        if (event.type === "animationcancel" &&
            Array.from(event.target?.getAnimations?.() || []).some(animation =>
              animation.animationName === event.animationName && animation.playState === "running")) return;
        node.classList.remove(poseClass);
      };
      node.addEventListener("animationend", stopPose);
      node.addEventListener("animationcancel", stopPose);
    });
  }
  function onAgentState(station, previous, next) {
    if (!station) return;
    const app = station.closest("#aoControlRoom");
    if (!app || app.classList.contains("ao-motion-disabled") ||
        window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches) {
      station.classList.remove("ao-v2-arriving", "ao-v2-rising");
      return;
    }
    if (previous === next || previous === "unknown" || next === "unknown") return;
    station.classList.remove("ao-v2-arriving", "ao-v2-rising");
    if (next === "work" && previous !== "work") station.classList.add("ao-v2-arriving");
    else if (previous === "work" && next === "idle") station.classList.add("ao-v2-rising");
  }
  function clearTransitions(stations) {
    if (!stations) return;
    for (const station of stations.querySelectorAll(".ao-station")) {
      station.classList.remove("ao-v2-arriving", "ao-v2-rising", "ao-v2-interacting");
    }
  }
  window.AOOfficeV2 = Object.freeze({character, decorate, onAgentState, clearTransitions});
})();