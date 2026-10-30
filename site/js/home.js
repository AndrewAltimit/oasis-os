// OASIS_OS landing page. No dependencies; everything degrades to static HTML.
(function () {
  "use strict";

  var reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  // ------------------------------------------------------------------
  // Top bar: border on scroll + mobile menu
  // ------------------------------------------------------------------
  var topbar = document.getElementById("topbar");
  var toggle = document.getElementById("nav-toggle");
  var nav = document.getElementById("site-nav");

  function onScroll() {
    if (topbar) topbar.classList.toggle("is-scrolled", window.scrollY > 8);
  }
  window.addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  function setMenu(open) {
    if (!toggle || !nav) return;
    nav.classList.toggle("is-open", open);
    toggle.setAttribute("aria-expanded", String(open));
  }
  if (toggle && nav) {
    toggle.addEventListener("click", function () {
      setMenu(toggle.getAttribute("aria-expanded") !== "true");
    });
    nav.addEventListener("click", function (e) {
      if (e.target.closest("a")) setMenu(false);
    });
    document.addEventListener("keydown", function (e) {
      if (e.key === "Escape" && nav.classList.contains("is-open")) {
        setMenu(false);
        toggle.focus();
      }
    });
  }

  // ------------------------------------------------------------------
  // Generic roving-tabindex tablist (arrow keys / Home / End)
  // ------------------------------------------------------------------
  function tablist(list, onSelect) {
    var tabs = Array.prototype.slice.call(list.querySelectorAll('[role="tab"]'));
    function select(tab, focus) {
      tabs.forEach(function (t) {
        var on = t === tab;
        t.setAttribute("aria-selected", String(on));
        t.tabIndex = on ? 0 : -1;
        t.classList.toggle("is-active", on);
      });
      if (focus) tab.focus();
      onSelect(tab);
    }
    tabs.forEach(function (tab, i) {
      tab.addEventListener("click", function () { select(tab, false); });
      tab.addEventListener("keydown", function (e) {
        var next = null;
        if (e.key === "ArrowRight") next = tabs[(i + 1) % tabs.length];
        else if (e.key === "ArrowLeft") next = tabs[(i - 1 + tabs.length) % tabs.length];
        else if (e.key === "Home") next = tabs[0];
        else if (e.key === "End") next = tabs[tabs.length - 1];
        if (next) { e.preventDefault(); select(next, true); }
      });
    });
  }

  // ------------------------------------------------------------------
  // Skin gallery
  // ------------------------------------------------------------------
  var gallery = document.getElementById("gallery");
  if (gallery) {
    var img = document.getElementById("stage-img");
    var list = document.getElementById("skin-list");
    var skins = Array.prototype.slice.call(list.querySelectorAll('[role="option"]'));
    var name = document.getElementById("stage-name");
    var desc = document.getElementById("stage-desc");
    var res = document.getElementById("stage-res");
    var src = document.getElementById("stage-src");
    var demo = document.getElementById("stage-demo");
    var state = { skin: skins[0], view: "01_dashboard" };
    var focusIdx = 0;

    function render() {
      var s = state.skin;
      var url = "screenshots/" + s.dataset.skin + "/" + state.view + ".png";
      var dims = s.dataset.res.split("×");
      var w = parseInt(dims[0], 10);
      var viewLabel = gallery.querySelector('.view[data-view="' + state.view + '"]');
      img.classList.add("is-loading");
      var pre = new Image();
      pre.onload = pre.onerror = function () {
        img.src = url;
        img.width = w;
        img.height = parseInt(dims[1], 10);
        // Upscale pixel-art captures crisply; leave large captures smooth.
        img.classList.toggle("is-small", w <= 640);
        img.alt = s.dataset.name + " skin, " + (viewLabel ? viewLabel.textContent : "") + " view";
        img.classList.remove("is-loading");
      };
      pre.src = url;
      name.textContent = s.dataset.name;
      desc.textContent = s.dataset.desc;
      res.textContent = s.dataset.res;
      src.textContent = s.dataset.src;
      demo.href = "demo/?skin=" + encodeURIComponent(s.dataset.skin);
    }

    function pick(s) {
      skins.forEach(function (o) {
        var on = o === s;
        o.classList.toggle("is-active", on);
        o.setAttribute("aria-selected", String(on));
      });
      state.skin = s;
      render();
    }

    function visibleSkins() {
      return skins.filter(function (s) { return !s.hidden; });
    }

    function setFocus(i) {
      var vis = visibleSkins();
      if (!vis.length) return;
      focusIdx = Math.max(0, Math.min(i, vis.length - 1));
      skins.forEach(function (s) { s.classList.remove("is-focused"); });
      var el = vis[focusIdx];
      el.classList.add("is-focused");
      list.setAttribute("aria-activedescendant", el.id);
      el.scrollIntoView({ block: "nearest", behavior: reduceMotion ? "auto" : "smooth" });
    }

    skins.forEach(function (s) {
      s.addEventListener("click", function () {
        pick(s);
        focusIdx = visibleSkins().indexOf(s);
      });
    });

    list.addEventListener("focus", function () {
      var vis = visibleSkins();
      var i = vis.indexOf(state.skin);
      setFocus(i < 0 ? 0 : i);
    });
    list.addEventListener("blur", function () {
      skins.forEach(function (s) { s.classList.remove("is-focused"); });
    });
    list.addEventListener("keydown", function (e) {
      var vis = visibleSkins();
      // Items per visual row: 1 in the horizontal (mobile) strip, else count the first row.
      var cols = 1;
      if (getComputedStyle(list).gridAutoFlow.indexOf("column") === -1 && vis.length) {
        var top = vis[0].offsetTop;
        cols = vis.filter(function (s) { return s.offsetTop === top; }).length || 1;
      }
      var map = { ArrowRight: 1, ArrowDown: cols, ArrowLeft: -1, ArrowUp: -cols };
      if (e.key in map) {
        e.preventDefault();
        setFocus(focusIdx + map[e.key]);
        pick(vis[focusIdx]);
      } else if (e.key === "Home" || e.key === "End") {
        e.preventDefault();
        setFocus(e.key === "Home" ? 0 : vis.length - 1);
        pick(vis[focusIdx]);
      }
    });

    tablist(gallery.querySelector(".views"), function (tab) {
      state.view = tab.dataset.view;
      render();
    });

    // Category filter chips
    var chips = Array.prototype.slice.call(gallery.querySelectorAll(".chip"));
    chips.forEach(function (chip) {
      chip.addEventListener("click", function () {
        var f = chip.dataset.filter;
        chips.forEach(function (c) {
          var on = c === chip;
          c.classList.toggle("is-active", on);
          c.setAttribute("aria-pressed", String(on));
        });
        skins.forEach(function (s) {
          s.hidden = f !== "all" && s.dataset.cat !== f;
        });
        if (state.skin.hidden) {
          var first = visibleSkins()[0];
          if (first) pick(first);
        }
      });
    });

    // Deep link: index.html?skin=xp#skins
    var want = new URLSearchParams(window.location.search).get("skin");
    var match = want && skins.filter(function (s) { return s.dataset.skin === want; })[0];
    if (match) pick(match);
  }

  // ------------------------------------------------------------------
  // Build console tabs + copy buttons
  // ------------------------------------------------------------------
  var consoleTabs = document.querySelector(".console__tabs");
  if (consoleTabs) {
    tablist(consoleTabs, function (tab) {
      var panels = document.querySelectorAll(".console__panel");
      Array.prototype.forEach.call(panels, function (p) {
        p.hidden = p.id !== tab.getAttribute("aria-controls");
      });
    });
  }

  Array.prototype.forEach.call(document.querySelectorAll("[data-copy]"), function (btn) {
    btn.addEventListener("click", function () {
      var code = btn.parentElement.querySelector("code");
      // Copy commands only: drop comment lines and blank lines.
      var text = code.textContent
        .split("\n")
        .filter(function (l) { return l.trim() && l.trim().charAt(0) !== "#"; })
        .join("\n");
      function done(ok) {
        btn.textContent = ok ? "Copied" : "Press Ctrl+C";
        btn.classList.toggle("is-done", ok);
        setTimeout(function () { btn.textContent = "Copy"; btn.classList.remove("is-done"); }, 1800);
      }
      if (navigator.clipboard && window.isSecureContext) {
        navigator.clipboard.writeText(text).then(function () { done(true); }, function () { done(false); });
      } else {
        var range = document.createRange();
        range.selectNodeContents(code);
        var sel = window.getSelection();
        sel.removeAllRanges();
        sel.addRange(range);
        done(false);
      }
    });
  });

  // ------------------------------------------------------------------
  // Reveal-on-scroll (skipped entirely for reduced motion)
  // ------------------------------------------------------------------
  if (!reduceMotion && "IntersectionObserver" in window) {
    var targets = document.querySelectorAll(
      ".section__head, .pillar, .platform, .gallery, .tile, .arch, .psp__copy, .psp__shots, .entries, .blueprint, .console"
    );
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (en.isIntersecting) {
          en.target.classList.add("is-in");
          io.unobserve(en.target);
        }
      });
    }, { rootMargin: "0px 0px -8% 0px", threshold: 0.05 });
    Array.prototype.forEach.call(targets, function (el) {
      // Only animate things below the fold, so nothing visible "pops".
      if (el.getBoundingClientRect().top > window.innerHeight) {
        el.classList.add("reveal");
        io.observe(el);
      }
    });
  }

  // Pause the hero video when offscreen or when the user prefers reduced motion.
  var video = document.querySelector(".screen__video");
  if (video) {
    if (reduceMotion) {
      video.removeAttribute("autoplay");
      video.pause();
      video.controls = true;
    } else if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (entries) {
        entries.forEach(function (en) {
          if (en.isIntersecting) { var p = video.play(); if (p && p.catch) p.catch(function () {}); }
          else video.pause();
        });
      }).observe(video);
    }
  }
})();
