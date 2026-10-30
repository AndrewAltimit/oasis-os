// OASIS_OS journal articles: reading progress, table-of-contents highlight,
// and Mermaid diagrams (loaded from a CDN only on pages that have one).
(function () {
  "use strict";

  // Reading progress (a transform, so it never triggers layout).
  var bar = document.getElementById("progress-bar");
  var prose = document.querySelector(".prose");
  if (bar && prose) {
    var ticking = false;
    var update = function () {
      ticking = false;
      var r = prose.getBoundingClientRect();
      var total = r.height - window.innerHeight * 0.6;
      var p = total > 0 ? Math.min(1, Math.max(0, -r.top / total)) : 1;
      bar.style.transform = "scaleX(" + p.toFixed(4) + ")";
    };
    var request = function () {
      if (!ticking) { ticking = true; window.requestAnimationFrame(update); }
    };
    window.addEventListener("scroll", request, { passive: true });
    window.addEventListener("resize", request);
    update();
  }

  // Highlight the section currently being read in the TOC.
  var links = Array.prototype.slice.call(document.querySelectorAll(".toc a"));
  if (links.length && "IntersectionObserver" in window) {
    var byId = {};
    links.forEach(function (a) { byId[a.getAttribute("href").slice(1)] = a; });
    var heads = Array.prototype.slice.call(document.querySelectorAll(".prose h2[id]"));
    var current = null;
    var setCurrent = function (id) {
      if (id === current) return;
      current = id;
      links.forEach(function (a) {
        var on = a === byId[id];
        a.classList.toggle("is-current", on);
        if (on) a.setAttribute("aria-current", "location");
        else a.removeAttribute("aria-current");
      });
    };
    var io = new IntersectionObserver(function () {
      // The current section is the last heading above the top third of the viewport.
      var line = window.innerHeight * 0.33;
      var pick = heads[0];
      heads.forEach(function (h) { if (h.getBoundingClientRect().top < line) pick = h; });
      if (pick) setCurrent(pick.id);
    }, { rootMargin: "0px 0px -60% 0px", threshold: [0, 1] });
    heads.forEach(function (h) { io.observe(h); });
  }

  // Mermaid: only fetched when the page contains a diagram.
  if (document.querySelector("pre.mermaid")) {
    import("https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs")
      .then(function (m) {
        var mermaid = m.default;
        mermaid.initialize({
          startOnLoad: false,
          theme: "base",
          fontFamily: "JetBrains Mono, ui-monospace, monospace",
          themeVariables: {
            darkMode: true,
            background: "#11141c",
            primaryColor: "#161a24",
            primaryTextColor: "#e8eaf0",
            primaryBorderColor: "#2f3548",
            lineColor: "#6d7486",
            secondaryColor: "#161a24",
            tertiaryColor: "#11141c",
            fontSize: "13px"
          }
        });
        return mermaid.run({ querySelector: "pre.mermaid" });
      })
      .catch(function (err) {
        // Offline or CDN blocked: the diagram source stays readable as text.
        if (window.console) console.warn("Mermaid unavailable:", err && err.message);
      });
  }
})();
