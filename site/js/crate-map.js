// OASIS_OS crate map (crates.html).
//
// Data: data/crates.json, generated from every crates/*/Cargo.toml by
// scripts/gen-crate-map.py. Nothing here knows a crate name: the bands,
// edges, detail card and walks all come from the JSON.
//
// Interaction:
//   hover / focus a crate   light what it uses and what uses it, draw edges
//   click / Enter           pin it (detail card, #crate=<name> in the URL)
//   Left / Right / Home / End  move between crates; Escape clears
//   toolbar                 search dims non-matches; chips toggle optional /
//                           dev edges, the PSP crates and transitive reach
//   walks                   step through a data path; "Show on the map"
//                           carries the trace into the graph
// Classic script, no dependencies; loaded with `defer`.
(function () {
  "use strict";

  var SVGNS = "http://www.w3.org/2000/svg";
  var REDUCED = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  var WIDE_EDGES = window.matchMedia("(min-width: 761px)");
  var TWO_COLUMN = window.matchMedia("(min-width: 1100px)");

  var data = null;
  var repo = "https://github.com/AndrewAltimit/oasis-os";
  var byName = {};
  var layerById = {};
  var out = {}; // name -> [{name, kind, features?}]  (what it uses)
  var inn = {}; // name -> [{name, kind, features?}]  (what uses it)
  var chips = {};
  var order = []; // chip names in map order
  var filters = { optional: true, dev: false, psp: true, transitive: false };
  var query = "";
  var selected = null;
  var hovered = null;
  var walk = null;
  var walkIndex = 0;
  var trace = false;
  var playTimer = null;
  var defaultCard = "";

  function $(id) { return document.getElementById(id); }
  function esc(s) {
    return String(s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }
  function fmt(n) { return n.toLocaleString("en-US"); }
  function label(name) {
    return name.indexOf("oasis-") === 0
      ? '<span class="cm-chip__pre">oasis-</span>' + esc(name.slice(6))
      : esc(name);
  }
  function scrollOpts(block) { return { behavior: REDUCED ? "auto" : "smooth", block: block || "start" }; }

  /* ---------- Graph helpers ---------- */

  function edgeOn(kind) {
    if (kind === "optional") return filters.optional;
    if (kind === "dev") return filters.dev;
    return true;
  }
  function crateOn(name) { return filters.psp || byName[name].target !== "psp"; }
  function visible(list) {
    return list.filter(function (e) { return edgeOn(e.kind) && crateOn(e.name); });
  }
  // Every crate reachable along `map` edges (visible ones, or all but dev).
  function closure(start, map, all) {
    var seen = {}, stack = [start];
    while (stack.length) {
      var cur = stack.pop();
      var next = all
        ? map[cur].filter(function (e) { return e.kind !== "dev"; })
        : visible(map[cur]);
      for (var i = 0; i < next.length; i++) {
        var n = next[i].name;
        if (!seen[n] && n !== start) { seen[n] = true; stack.push(n); }
      }
    }
    return seen;
  }
  function related(a, b) {
    return out[a].some(function (e) { return e.name === b; }) ||
      out[b].some(function (e) { return e.name === a; });
  }

  /* ---------- Build ---------- */

  function buildBands() {
    var host = $("cm-bands");
    host.innerHTML = "";
    data.layers.forEach(function (layer) {
      var names = data.crates.filter(function (c) { return c.layer === layer.id; })
        .map(function (c) { return c.name; });
      var band = document.createElement("div");
      band.className = "cm-band cm-layer--" + layer.id + (layer.id === "backends" ? " cm-band--boundary" : "");
      band.setAttribute("role", "group");
      band.setAttribute("aria-labelledby", "cm-band-" + layer.id);
      band.dataset.layer = layer.id;
      band.innerHTML =
        '<div class="cm-band__head"><p class="cm-band__title" id="cm-band-' + layer.id + '">' +
        esc(layer.title) + "<span>" + names.length + "</span></p>" +
        '<p class="cm-band__blurb">' + esc(layer.blurb) + "</p></div>" +
        '<div class="cm-band__chips"></div>';
      var row = band.querySelector(".cm-band__chips");
      names.forEach(function (name) {
        var c = byName[name];
        var b = document.createElement("button");
        b.type = "button";
        b.className = "cm-chip";
        b.dataset.crate = name;
        b.setAttribute("aria-pressed", "false");
        b.setAttribute("aria-label", name + ": " + c.description + ". Uses " + out[name].length +
          ", used by " + inn[name].length + ".");
        b.innerHTML = label(name) +
          (c.target === "psp" && c.layer !== "psp" ? '<span class="cm-chip__tag">psp</span>' : "");
        b.addEventListener("mouseenter", function () { hover(name); });
        b.addEventListener("mouseleave", function () { hover(null); });
        b.addEventListener("focus", function () { hover(name); });
        b.addEventListener("blur", function () { hover(null); });
        b.addEventListener("click", function () { select(selected === name ? null : name, true); });
        b.addEventListener("keydown", onChipKey);
        chips[name] = b;
        order.push(name);
        row.appendChild(b);
      });
      host.appendChild(band);
    });
  }

  function onChipKey(e) {
    var keys = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1, Home: "first", End: "last" };
    var k = keys[e.key];
    if (!k) return;
    var shown = order.filter(function (n) { return !chips[n].hidden; });
    var i = shown.indexOf(e.currentTarget.dataset.crate);
    var j = k === "first" ? 0 : k === "last" ? shown.length - 1 : i + k;
    if (j < 0 || j >= shown.length) return;
    e.preventDefault();
    chips[shown[j]].focus();
  }

  /* ---------- Highlight + edges ---------- */

  var STATES = ["is-focus", "is-sel", "is-uses", "is-usedby", "is-soft", "is-far", "is-walk", "is-trail"];

  function render() {
    var root = $("cm-root");
    var svg = $("cm-edges");
    for (var n in chips) {
      chips[n].classList.remove.apply(chips[n].classList, STATES);
      chips[n].setAttribute("aria-pressed", n === selected ? "true" : "false");
    }
    svg.innerHTML = "";
    root.classList.remove("has-focus");
    if (trace && !hovered) { renderTrace(); return; }
    var focus = hovered || selected;
    if (!focus) return;
    root.classList.add("has-focus");
    chips[focus].classList.add("is-focus");
    if (selected) chips[selected].classList.add("is-sel");
    if (filters.transitive) {
      var far = closure(focus, out, false), back = closure(focus, inn, false);
      for (var a in far) chips[a].classList.add("is-far");
      for (var b in back) chips[b].classList.add("is-far");
    }
    var uses = visible(out[focus]), usedBy = visible(inn[focus]);
    uses.forEach(function (e) { mark(e, "is-uses"); });
    usedBy.forEach(function (e) { mark(e, "is-usedby"); });
    if (!WIDE_EDGES.matches) return;
    var frame = frameRect();
    uses.forEach(function (e) { edge(svg, frame, chips[focus], chips[e.name], "uses " + e.kind); });
    usedBy.forEach(function (e) { edge(svg, frame, chips[e.name], chips[focus], "usedby " + e.kind); });
  }

  function mark(e, cls) {
    var chip = chips[e.name];
    chip.classList.remove("is-far");
    chip.classList.add(cls);
    if (e.kind !== "normal") chip.classList.add("is-soft");
  }

  function renderTrace() {
    var steps = walk.steps;
    $("cm-root").classList.add("has-focus");
    for (var i = 0; i < walkIndex; i++) {
      steps[i].crates.forEach(function (n) { if (chips[n]) chips[n].classList.add("is-trail"); });
    }
    var cur = steps[walkIndex].crates;
    cur.forEach(function (n) { if (chips[n]) chips[n].classList.add("is-walk"); });
    if (!walkIndex || !WIDE_EDGES.matches) return;
    var svg = $("cm-edges"), frame = frameRect();
    var prev = steps[walkIndex - 1].crates;
    var drawn = 0;
    // Join each crate of the last step to each crate of this one it has a
    // dependency edge with; if none is related, join first to first so the
    // path stays continuous.
    prev.forEach(function (p) {
      cur.forEach(function (q) {
        if (p !== q && related(p, q)) { edge(svg, frame, chips[p], chips[q], "walk"); drawn++; }
      });
    });
    if (!drawn && prev[0] !== cur[0]) edge(svg, frame, chips[prev[0]], chips[cur[0]], "walk");
  }

  function frameRect() {
    var r = $("cm-stage").getBoundingClientRect();
    $("cm-edges").setAttribute("viewBox", "0 0 " + Math.max(1, r.width) + " " + Math.max(1, r.height));
    return r;
  }

  function clamp(v, lo, hi) { return Math.max(lo, Math.min(hi, v)); }

  function edge(svg, frame, from, to, cls) {
    if (!from || !to || from.hidden || to.hidden) return;
    var a = from.getBoundingClientRect(), b = to.getBoundingClientRect();
    // Each end leaves its chip at the point nearest the other end, so a
    // crate with many edges fans them out along its edge.
    var acx = a.left + a.width / 2, bcx = b.left + b.width / 2;
    var ax = clamp(bcx, a.left + 8, a.right - 8) - frame.left;
    var bx = clamp(acx, b.left + 8, b.right - 8) - frame.left;
    var ay, by, d;
    if (Math.abs(a.top - b.top) < 4) {
      ay = a.top - frame.top; by = b.top - frame.top;
      var lift = Math.min(44, 14 + Math.abs(ax - bx) / 6);
      d = "M" + ax + "," + ay + " C" + ax + "," + (ay - lift) + " " + bx + "," + (by - lift) + " " + bx + "," + by;
    } else {
      var down = b.top > a.top;
      ay = (down ? a.bottom : a.top) - frame.top;
      by = (down ? b.top : b.bottom) - frame.top;
      var my = (ay + by) / 2;
      d = "M" + ax + "," + ay + " C" + ax + "," + my + " " + bx + "," + my + " " + bx + "," + by;
    }
    var p = document.createElementNS(SVGNS, "path");
    p.setAttribute("d", d);
    p.setAttribute("class", cls);
    svg.appendChild(p);
  }

  /* ---------- Filters ---------- */

  function matches(name) {
    if (!query) return true;
    var c = byName[name];
    return name.indexOf(query) >= 0 || c.description.toLowerCase().indexOf(query) >= 0 ||
      layerById[c.layer].title.toLowerCase().indexOf(query) >= 0;
  }

  function applyFilters() {
    var shown = 0, hits = 0, edges = 0;
    order.forEach(function (n) {
      var on = crateOn(n);
      chips[n].hidden = !on;
      var hit = on && matches(n);
      chips[n].classList.toggle("is-off", on && !hit);
      if (on) { shown++; edges += visible(out[n]).length; }
      if (hit) hits++;
    });
    document.querySelectorAll(".cm-band").forEach(function (band) {
      band.hidden = !band.querySelector(".cm-chip:not([hidden])");
    });
    var total = order.length;
    var text = (shown === total ? total : shown + " of " + total) + " crates · " + edges + " edges";
    if (query) text = hits + " match" + (hits === 1 ? "" : "es") + " · " + text;
    $("cm-count").textContent = text;
    if (selected && !crateOn(selected)) { select(null); return; }
    if (hovered && !crateOn(hovered)) hovered = null;
    placeCard();
  }

  function buildToolbar() {
    document.querySelectorAll(".cm-toggles [data-filter]").forEach(function (b) {
      b.addEventListener("click", function () {
        var f = b.dataset.filter;
        filters[f] = !filters[f];
        b.classList.toggle("is-active", filters[f]);
        b.setAttribute("aria-pressed", String(filters[f]));
        applyFilters();
      });
    });
    $("cm-search").addEventListener("input", function (e) {
      query = e.target.value.trim().toLowerCase();
      applyFilters();
    });
  }

  /* ---------- Detail card ---------- */

  var MEMBERSHIP = {
    member: "workspace member",
    excluded: "excluded · mipsel-sony-psp",
    standalone: "standalone · cargo psp",
  };

  function mini(list, cls) {
    if (!list.length) return '<p class="cm-none">none</p>';
    return '<div class="cm-mini ' + cls + '">' + list.map(function (e) {
      var note = e.kind === "optional" && e.features && e.features.length
        ? "<small>" + esc(e.features[0]) + (e.features.length > 1 ? " +" + (e.features.length - 1) : "") + "</small>"
        : "";
      var title = e.kind === "optional" ? ' title="optional, enabled by: ' + esc((e.features || []).join(", ")) + '"' : "";
      return '<button type="button" data-go="' + esc(e.name) + '"' + title + ">" + esc(e.name) + note + "</button>";
    }).join("") + "</div>";
  }

  function grouped(list, cls) {
    var groups = [["normal", ""], ["build", "Build"], ["optional", "Optional (feature-gated)"], ["dev", "Dev / tests only"]];
    var html = "";
    groups.forEach(function (g) {
      var part = list.filter(function (e) { return e.kind === g[0]; });
      if (!part.length) return;
      if (g[1]) html += '<p class="cm-sub">' + g[1] + "</p>";
      html += mini(part, cls);
    });
    return html || '<p class="cm-none">none</p>';
  }

  function kindOf(c) {
    if (c.crate_type.indexOf("cdylib") >= 0) return "cdylib";
    if (c.lib && c.bins.length) return "lib + " + c.bins.length + " bin";
    if (c.lib) return "lib";
    return "bin";
  }

  function walksWith(name) {
    var hits = [];
    data.walks.forEach(function (w) {
      w.steps.forEach(function (s, i) {
        if (s.crates.indexOf(name) >= 0) hits.push({ walk: w, index: i });
      });
    });
    return hits;
  }

  function crateCard(name) {
    var c = byName[name], layer = layerById[c.layer];
    var pulls = Object.keys(closure(name, out, true)).length;
    var feeds = Object.keys(closure(name, inn, true)).length;
    var h = '<p class="cm-card__kicker">Crate</p><p class="cm-card__name">' + esc(name) + "</p>";
    h += '<div class="cm-card__tags"><span class="cm-tag cm-layer--' + c.layer + '">' + esc(layer.title) + "</span>" +
      '<span class="cm-tag">' + esc(MEMBERSHIP[c.membership] || c.membership) + "</span></div>";
    h += '<p class="cm-card__desc">' + esc(c.description) + "</p>";
    h += '<dl class="cm-facts"><div><dt>Rust lines</dt><dd>' + fmt(c.loc) + "</dd></div>" +
      "<div><dt>Files</dt><dd>" + fmt(c.files) + "</dd></div>" +
      "<div><dt>Kind</dt><dd>" + esc(kindOf(c)) + "</dd></div></dl>";
    h += '<p class="cm-h cm-h--uses">Uses (' + out[name].length + ")</p>" + grouped(out[name], "cm-mini--uses");
    h += '<p class="cm-h cm-h--usedby">Used by (' + inn[name].length + ")</p>" + grouped(inn[name], "cm-mini--usedby");
    h += '<p class="cm-reach">Pulls in <strong>' + pulls + "</strong> crate" + (pulls === 1 ? "" : "s") +
      " transitively; <strong>" + feeds + "</strong> depend on it, directly or not (dev-dependencies aside).</p>";
    var w = walksWith(name);
    if (w.length) {
      h += '<p class="cm-h cm-h--walk">In the walks</p><div class="cm-mini cm-mini--walk">' + w.map(function (x) {
        return '<button type="button" data-walk-go="' + esc(x.walk.id) + ":" + x.index + '">' +
          esc(x.walk.title) + "<small>step " + (x.index + 1) + "</small></button>";
      }).join("") + "</div>";
    }
    if (c.features.length) {
      h += '<p class="cm-h">Features (' + c.features.length + ')</p><p class="cm-none mono">' +
        c.features.map(esc).join(" · ") + "</p>";
    }
    h += '<div class="cm-links">' +
      '<a class="btn btn--ghost" href="' + repo + "/tree/main/" + esc(c.dir) + '">Source on GitHub</a>' +
      '<a class="btn btn--quiet" href="' + repo + "/blob/main/" + esc(c.dir) + '/Cargo.toml">Cargo.toml</a>' +
      '<button type="button" class="btn btn--quiet" data-clear="1">Clear</button></div>';
    return h;
  }

  function traceCard() {
    var s = walk.steps[walkIndex];
    return '<p class="cm-card__kicker">' + esc(walk.title) + " · step " + (walkIndex + 1) + " of " +
      walk.steps.length + '</p><p class="cm-card__name">' + esc(s.title) + "</p>" +
      '<p class="cm-card__desc">' + esc(s.text) + "</p>" +
      '<p class="cm-h cm-h--walk">Crates</p>' + mini(s.crates.map(function (n) { return { name: n, kind: "normal" }; }), "cm-mini--walk") +
      '<div class="cm-links">' +
      '<button type="button" class="btn btn--ghost" data-step="-1"' + (walkIndex === 0 ? " disabled" : "") + ">Previous</button>" +
      '<button type="button" class="btn btn--ghost" data-step="1"' + (walkIndex === walk.steps.length - 1 ? " disabled" : "") + ">Next</button>" +
      '<button type="button" class="btn btn--quiet" data-clear="1">End trace</button></div>';
  }

  function renderCard() {
    var card = $("cm-card");
    if (trace) card.innerHTML = traceCard();
    else if (selected) card.innerHTML = crateCard(selected);
    else card.innerHTML = defaultCard;
  }

  function onCardClick(e) {
    var t = e.target.closest("[data-go],[data-clear],[data-step],[data-walk-go]");
    if (!t) return;
    if (t.dataset.go) {
      trace = false;
      select(t.dataset.go, false);
      if (chips[t.dataset.go]) chips[t.dataset.go].focus({ preventScroll: true });
    } else if (t.dataset.clear) {
      stopPlay();
      trace = false;
      select(null, false);
    } else if (t.dataset.step) {
      stopPlay();
      setStep(walkIndex + parseInt(t.dataset.step, 10));
    } else if (t.dataset.walkGo) {
      var parts = t.dataset.walkGo.split(":");
      setWalk(parts[0], parseInt(parts[1], 10));
      $("walks").scrollIntoView(scrollOpts());
    }
  }

  /* ---------- Selection ---------- */

  function hover(name) {
    if (hovered === name) return;
    hovered = name;
    render();
  }

  function select(name, fromChip) {
    selected = name && byName[name] && crateOn(name) ? name : null;
    if (selected) trace = false;
    try {
      history.replaceState(null, "", selected ? "#crate=" + selected : location.pathname + location.search);
    } catch (err) { /* file:// or a sandboxed frame */ }
    renderCard();
    placeCard();
    if (selected && fromChip && !TWO_COLUMN.matches) {
      $("cm-card").scrollIntoView(scrollOpts("nearest"));
    }
  }

  // In one column the card would sit after every band, a long scroll from
  // the crate that filled it: there it follows the focused crate's band.
  function placeCard() {
    var card = $("cm-card");
    var focus = selected || (trace ? walk.steps[walkIndex].crates[0] : null);
    if (!TWO_COLUMN.matches && focus && chips[focus]) {
      var band = chips[focus].closest(".cm-band");
      if (band && band.nextElementSibling !== card) band.parentNode.insertBefore(card, band.nextSibling);
    } else if (card.parentNode !== $("cm-body")) {
      $("cm-body").appendChild(card);
    }
    render();
  }

  /* ---------- Walks ---------- */

  function buildWalks() {
    var tabs = $("cm-walk-tabs");
    data.walks.forEach(function (w, i) {
      var b = document.createElement("button");
      b.type = "button";
      b.id = "cm-tab-" + w.id;
      b.setAttribute("role", "tab");
      b.setAttribute("aria-controls", "cm-walk-body");
      b.textContent = w.title;
      b.addEventListener("click", function () { stopPlay(); setWalk(w.id, 0); });
      b.addEventListener("keydown", function (e) {
        var d = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
        if (!d) return;
        e.preventDefault();
        var next = data.walks[(i + d + data.walks.length) % data.walks.length];
        stopPlay();
        setWalk(next.id, 0);
        $("cm-tab-" + next.id).focus();
      });
      tabs.appendChild(b);
    });
    $("cm-step").addEventListener("click", function (e) {
      var t = e.target.closest("[data-go],[data-act]");
      if (!t) return;
      if (t.dataset.go) {
        trace = false;
        select(t.dataset.go, false);
        $("map").scrollIntoView(scrollOpts());
        return;
      }
      var act = t.dataset.act;
      if (act === "prev") { stopPlay(); setStep(walkIndex - 1); }
      else if (act === "next") { stopPlay(); setStep(walkIndex + 1); }
      else if (act === "play") { if (playTimer) stopPlay(); else startPlay(); renderStep(); }
      else if (act === "trace") {
        selected = null;
        trace = true;
        renderCard();
        placeCard();
        $("map").scrollIntoView(scrollOpts());
      }
    });
    setWalk(data.walks[0].id, 0);
  }

  function setWalk(id, index) {
    walk = data.walks.filter(function (w) { return w.id === id; })[0] || data.walks[0];
    data.walks.forEach(function (w) {
      var tab = $("cm-tab-" + w.id);
      var on = w === walk;
      tab.setAttribute("aria-selected", String(on));
      tab.tabIndex = on ? 0 : -1;
    });
    $("cm-walk-body").setAttribute("aria-labelledby", "cm-tab-" + walk.id);
    var list = $("cm-steps");
    list.innerHTML = "";
    walk.steps.forEach(function (s, i) {
      var li = document.createElement("li");
      var b = document.createElement("button");
      b.type = "button";
      b.innerHTML = '<span class="cm-steps__n" aria-hidden="true">' + (i + 1) + "</span>" + esc(s.title);
      b.addEventListener("click", function () { stopPlay(); setStep(i); });
      li.appendChild(b);
      list.appendChild(li);
    });
    setStep(index || 0);
  }

  function setStep(i) {
    walkIndex = Math.max(0, Math.min(walk.steps.length - 1, i));
    var buttons = $("cm-steps").querySelectorAll("button");
    for (var k = 0; k < buttons.length; k++) {
      if (k === walkIndex) buttons[k].setAttribute("aria-current", "step");
      else buttons[k].removeAttribute("aria-current");
      buttons[k].classList.toggle("is-done", k < walkIndex);
    }
    var cur = buttons[walkIndex];
    if (cur && !WIDE_EDGES.matches) {
      // The step list scrolls sideways on phones: keep the current one in view.
      var strip = $("cm-steps");
      strip.scrollLeft = cur.parentNode.offsetLeft - strip.clientWidth / 2 + cur.offsetWidth / 2;
    }
    renderStep();
    if (trace) { renderCard(); placeCard(); }
  }

  function codeLink(ref) {
    var at = ref.indexOf(": ");
    var path = at >= 0 ? ref.slice(0, at) : ref;
    var item = at >= 0 ? ref.slice(at + 2) : "";
    var kind = /\.[a-z]+$/.test(path) ? "blob" : "tree";
    return '<a href="' + repo + "/" + kind + "/main/" + esc(path) + '">' + esc(path) +
      (item ? " <span>&middot; " + esc(item) + "</span>" : "") + "</a>";
  }

  function renderStep() {
    var s = walk.steps[walkIndex], n = walk.steps.length;
    var strip = walk.steps.map(function (x, i) {
      return '<span class="' + (i === walkIndex ? "is-on" : i < walkIndex ? "is-done" : "") + '"></span>';
    }).join("");
    $("cm-step").innerHTML =
      '<div class="cm-step__strip" aria-hidden="true">' + strip + "</div>" +
      '<p class="cm-step__meta">' + esc(walk.title.toUpperCase()) + " · STEP " + (walkIndex + 1) + " OF " + n + "</p>" +
      '<h3 class="cm-step__title">' + esc(s.title) + "</h3>" +
      '<p class="cm-step__text">' + esc(s.text) + "</p>" +
      '<p class="cm-h cm-h--walk">Crates</p>' +
      mini(s.crates.map(function (c) { return { name: c, kind: "normal" }; }), "cm-mini--walk") +
      '<p class="cm-h">Code</p><div class="cm-code">' + s.code.map(codeLink).join("") + "</div>" +
      '<div class="cm-step__ctl">' +
      '<button type="button" class="btn btn--ghost" data-act="prev"' + (walkIndex === 0 ? " disabled" : "") + ">Previous</button>" +
      '<button type="button" class="btn btn--ghost" data-act="next"' + (walkIndex === n - 1 ? " disabled" : "") + ">Next</button>" +
      '<button type="button" class="btn btn--quiet" data-act="play" aria-pressed="' + (playTimer ? "true" : "false") + '">' +
      (playTimer ? "Pause" : "Play the path") + "</button>" +
      '<button type="button" class="btn btn--primary" data-act="trace">Show on the map</button></div>';
  }

  function startPlay() {
    if (walkIndex === walk.steps.length - 1) setStep(0);
    playTimer = setInterval(function () {
      if (walkIndex >= walk.steps.length - 1) { stopPlay(); renderStep(); return; }
      setStep(walkIndex + 1);
    }, 3200);
  }
  function stopPlay() { if (playTimer) { clearInterval(playTimer); playTimer = null; } }

  /* ---------- Boot ---------- */

  function heroStats() {
    var edges = 0, loc = 0, psp = 0;
    data.crates.forEach(function (c) {
      edges += c.deps.filter(function (e) { return e.kind !== "dev"; }).length;
      loc += c.loc;
      if (c.target === "psp") psp++;
    });
    var items = [
      '<span class="dot" aria-hidden="true"></span>' + data.crates.length + " crates",
      (data.crates.length - psp) + " workspace · " + psp + " PSP",
      edges + " dependency edges",
      fmt(Math.round(loc / 1000)) + "k lines of Rust",
    ];
    $("cm-stats-hero").innerHTML = items.map(function (t) { return '<li class="badge">' + t + "</li>"; }).join("");
  }

  function fromHash() {
    var m = /^#crate=([a-z0-9-]+)$/.exec(location.hash || "");
    if (m && byName[m[1]]) {
      select(m[1], false);
      if (chips[m[1]]) chips[m[1]].scrollIntoView(scrollOpts("center"));
    }
  }

  function init(json) {
    data = json;
    repo = (data.repo || repo).replace(/\/$/, "");
    data.layers.forEach(function (l) { layerById[l.id] = l; });
    data.crates.forEach(function (c) { byName[c.name] = c; out[c.name] = []; inn[c.name] = []; });
    data.crates.forEach(function (c) {
      c.deps.forEach(function (e) {
        if (!byName[e.name]) return;
        out[c.name].push(e);
        inn[e.name].push({ name: c.name, kind: e.kind, features: e.features });
      });
    });
    data.crates.forEach(function (c) {
      inn[c.name].sort(function (a, b) { return a.name < b.name ? -1 : 1; });
    });
    defaultCard = $("cm-card").innerHTML;
    heroStats();
    buildBands();
    buildToolbar();
    buildWalks();
    applyFilters();
    $("cm-card").addEventListener("click", onCardClick);
    document.addEventListener("keydown", function (e) {
      if (e.key !== "Escape" || !(selected || trace)) return;
      if (document.querySelector(".site-nav.is-open")) return;
      stopPlay();
      trace = false;
      select(null, false);
    });
    var pending = false;
    var redraw = function () {
      if (pending) return;
      pending = true;
      requestAnimationFrame(function () { pending = false; placeCard(); });
    };
    if (window.ResizeObserver) new ResizeObserver(redraw).observe($("cm-stage"));
    else window.addEventListener("resize", redraw);
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(redraw);
    window.addEventListener("hashchange", fromHash);
    fromHash();
  }

  function boot() {
    fetch("data/crates.json")
      .then(function (r) {
        if (!r.ok) throw new Error("data/crates.json: HTTP " + r.status);
        return r.json();
      })
      .then(init)
      .catch(function (err) {
        $("cm-bands").innerHTML = '<p class="cm-empty">The crate map could not load (' +
          esc(err && err.message ? err.message : err) + '). The same graph is in <a href="' +
          repo + '/blob/main/CLAUDE.md#crate-dependency-graph">CLAUDE.md</a>.</p>';
      });
  }

  boot();
})();
