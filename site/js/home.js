// OASIS_OS landing page: code tabs, platform switcher, skin gallery,
// build console, scroll-spy, reveal-on-scroll, hero video.
// The top bar + mobile menu live in site.js. No dependencies; every
// section is plain static HTML first and degrades without JS.
(function () {
  "use strict";

  var reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  var $ = function (sel, root) { return (root || document).querySelector(sel); };
  var $$ = function (sel, root) {
    return Array.prototype.slice.call((root || document).querySelectorAll(sel));
  };

  // ------------------------------------------------------------------
  // Generic roving-tabindex tablist (arrow keys / Home / End). Panels are
  // found through aria-controls; `onSelect` runs after every change.
  // Returns a select(tab) function for programmatic use.
  // ------------------------------------------------------------------
  function tablist(list, onSelect) {
    var tabs = $$('[role="tab"]', list);
    var vertical = list.getAttribute("aria-orientation") === "vertical";
    function select(tab, focus) {
      tabs.forEach(function (t) {
        var on = t === tab;
        t.setAttribute("aria-selected", String(on));
        t.tabIndex = on ? 0 : -1;
        t.classList.toggle("is-active", on);
        var panel = document.getElementById(t.getAttribute("aria-controls"));
        if (panel) panel.hidden = !on;
      });
      if (focus) tab.focus();
      if (onSelect) onSelect(tab);
    }
    tabs.forEach(function (tab, i) {
      tab.addEventListener("click", function () { select(tab, false); });
      tab.addEventListener("keydown", function (e) {
        var next = null;
        var fwd = vertical ? ["ArrowDown", "ArrowRight"] : ["ArrowRight"];
        var back = vertical ? ["ArrowUp", "ArrowLeft"] : ["ArrowLeft"];
        if (fwd.indexOf(e.key) !== -1) next = tabs[(i + 1) % tabs.length];
        else if (back.indexOf(e.key) !== -1) next = tabs[(i - 1 + tabs.length) % tabs.length];
        else if (e.key === "Home") next = tabs[0];
        else if (e.key === "End") next = tabs[tabs.length - 1];
        if (next) { e.preventDefault(); select(next, true); }
      });
    });
    return select;
  }

  // ------------------------------------------------------------------
  // Code example tabs (01) and build console tabs (08)
  // ------------------------------------------------------------------
  var codeTabs = $("#contract-code .tabs");
  if (codeTabs) tablist(codeTabs);
  var consoleTabs = $(".console__tabs");
  if (consoleTabs) tablist(consoleTabs);

  // ------------------------------------------------------------------
  // Platform switcher (02). Without JS every panel shows, stacked.
  // ------------------------------------------------------------------
  var platforms = $("#platform-switch");
  var selectPlatform = null;
  if (platforms) {
    var ptabs = $(".platforms__tabs", platforms);
    ptabs.hidden = false;
    platforms.classList.add("is-enhanced");
    selectPlatform = tablist(ptabs);
    var first = $('[role="tab"]', ptabs);
    if (first) {
      // Hide every panel but the first, without stealing focus.
      $$('[role="tab"]', ptabs).forEach(function (t) {
        var p = document.getElementById(t.getAttribute("aria-controls"));
        if (p) p.hidden = t !== first;
      });
    }

    // "Runs on" links and #platform-<id> hashes select a platform.
    var openPlatform = function (id, scroll) {
      var tab = document.getElementById("ptab-" + id);
      if (!tab) return false;
      selectPlatform(tab, false);
      if (scroll) {
        document.getElementById("platforms").scrollIntoView({
          behavior: reduceMotion ? "auto" : "smooth", block: "start"
        });
      }
      return true;
    };
    $$(".runs-on a[data-platform]").forEach(function (a) {
      a.addEventListener("click", function (e) {
        if (openPlatform(a.dataset.platform, true)) {
          e.preventDefault();
          history.replaceState(null, "", "#platform-" + a.dataset.platform);
        }
      });
    });
    var hash = window.location.hash.match(/^#platform-([a-z0-9]+)$/);
    if (hash) openPlatform(hash[1], true);
  }

  // ------------------------------------------------------------------
  // Skin gallery (03)
  // ------------------------------------------------------------------
  var gallery = document.getElementById("gallery");
  if (gallery) initGallery(gallery);

  function initGallery(gallery) {
    var stage = $("#stage", gallery);
    var img = $("#stage-img", gallery);
    var list = $("#skin-list", gallery);
    var skins = $$('[role="option"]', list);
    var views = $$(".view", gallery);
    var els = {
      name: $("#stage-name", gallery),
      desc: $("#stage-desc", gallery),
      cat: $("#stage-cat", gallery),
      res: $("#stage-res", gallery),
      src: $("#stage-src", gallery),
      demo: $("#stage-demo", gallery),
      count: $("#stage-count", gallery),
      swatches: $("#stage-swatches", gallery),
      prev: $("#stage-prev", gallery),
      next: $("#stage-next", gallery)
    };
    var chips = $$(".chip", gallery);
    var catLabel = {};
    chips.forEach(function (c) {
      catLabel[c.dataset.filter] = c.firstChild.textContent.trim();
    });
    var state = { skin: skins[0], view: "01_dashboard" };
    var focusIdx = 0;
    var token = 0;
    var paletteCache = {};

    function visibleSkins() {
      return skins.filter(function (s) { return !s.hidden; });
    }

    function shot(skin, view) {
      return "screenshots/" + skin.dataset.skin + "/" + view + ".png";
    }

    // `quiet` skips the entrance animation (first paint, deep links).
    function render(quiet) {
      var s = state.skin;
      var url = shot(s, state.view);
      var dims = s.dataset.res.split("×");
      var w = parseInt(dims[0], 10);
      var h = parseInt(dims[1], 10);
      var viewTab = $('.view[data-view="' + state.view + '"]', gallery);
      var my = ++token;
      img.classList.add("is-loading");
      var pre = new Image();
      pre.onload = pre.onerror = function () {
        if (my !== token) return; // a newer pick won the race
        stage.dataset.device = w <= 480 ? "handheld" : "monitor";
        img.src = url;
        img.width = w;
        img.height = h;
        // Upscale pixel-art captures crisply; leave large captures smooth.
        img.classList.toggle("is-small", w <= 640);
        img.alt = s.dataset.name + " skin, " +
          (viewTab ? viewTab.textContent.trim() : "") + " view";
        img.classList.remove("is-loading");
        if (!reduceMotion && !quiet) {
          img.classList.remove("is-in");
          void img.offsetWidth; // restart the entrance animation
          img.classList.add("is-in");
        }
        palette(pre, url);
      };
      pre.src = url;

      els.name.textContent = s.dataset.name;
      els.desc.textContent = s.dataset.desc;
      els.cat.textContent = catLabel[s.dataset.cat] || "";
      els.res.textContent = s.dataset.res;
      els.src.textContent = s.dataset.src;
      els.src.href = "https://github.com/AndrewAltimit/oasis-os/tree/main/" + s.dataset.src;
      els.demo.href = "demo/?skin=" + encodeURIComponent(s.dataset.skin);
      var vis = visibleSkins();
      var n = vis.indexOf(s) + 1;
      els.count.textContent = pad(n) + " / " + pad(vis.length);
      views.forEach(function (v) {
        var thumb = $("img", v);
        if (thumb) thumb.src = shot(s, v.dataset.view);
      });
    }

    function pad(n) { return (n < 10 ? "0" : "") + n; }

    // Sample a small palette from the capture itself (no duplicated theme
    // data) and tint the stage glow with its most vivid color. Canvas reads
    // fail on file:// (tainted canvas); the stage just keeps its last glow.
    function palette(image, url) {
      var colors = paletteCache[url];
      if (!colors) {
        try {
          var cw = 48, ch = 28;
          var c = document.createElement("canvas");
          c.width = cw; c.height = ch;
          var ctx = c.getContext("2d", { willReadFrequently: true });
          ctx.drawImage(image, 0, 0, cw, ch);
          var data = ctx.getImageData(0, 0, cw, ch).data;
          var buckets = {};
          for (var i = 0; i < data.length; i += 4) {
            var key = (data[i] >> 5) + "," + (data[i + 1] >> 5) + "," + (data[i + 2] >> 5);
            var b = buckets[key] || (buckets[key] = { n: 0, r: 0, g: 0, b: 0 });
            b.n++; b.r += data[i]; b.g += data[i + 1]; b.b += data[i + 2];
          }
          var all = Object.keys(buckets).map(function (k) {
            var b = buckets[k];
            return [Math.round(b.r / b.n), Math.round(b.g / b.n), Math.round(b.b / b.n), b.n];
          }).sort(function (a, b) { return b[3] - a[3]; });
          colors = [];
          all.forEach(function (col) {
            if (colors.length >= 5) return;
            var distinct = colors.every(function (o) {
              return Math.abs(o[0] - col[0]) + Math.abs(o[1] - col[1]) + Math.abs(o[2] - col[2]) > 90;
            });
            if (distinct) colors.push(col);
          });
          paletteCache[url] = colors;
        } catch (err) {
          return;
        }
      }
      els.swatches.innerHTML = "";
      var vivid = null, best = -1;
      colors.forEach(function (col) {
        var li = document.createElement("li");
        li.style.setProperty("--c", "rgb(" + col[0] + "," + col[1] + "," + col[2] + ")");
        els.swatches.appendChild(li);
        var mx = Math.max(col[0], col[1], col[2]);
        var mn = Math.min(col[0], col[1], col[2]);
        var score = (mx - mn) * (mx / 255);
        if (score > best) { best = score; vivid = col; }
      });
      if (vivid && best > 30) {
        stage.style.setProperty("--skin-glow", "rgb(" + vivid[0] + "," + vivid[1] + "," + vivid[2] + ")");
      } else {
        stage.style.removeProperty("--skin-glow");
      }
    }

    function pick(s, quiet) {
      skins.forEach(function (o) {
        var on = o === s;
        o.classList.toggle("is-active", on);
        o.setAttribute("aria-selected", String(on));
      });
      list.setAttribute("aria-activedescendant", s.id);
      state.skin = s;
      render(quiet);
    }

    function step(delta) {
      var vis = visibleSkins();
      if (!vis.length) return;
      var i = vis.indexOf(state.skin);
      var next = vis[(i + delta + vis.length) % vis.length];
      pick(next);
      focusIdx = vis.indexOf(next);
      next.scrollIntoView({ block: "nearest", inline: "nearest", behavior: reduceMotion ? "auto" : "smooth" });
    }

    function setFocus(i) {
      var vis = visibleSkins();
      if (!vis.length) return;
      focusIdx = Math.max(0, Math.min(i, vis.length - 1));
      skins.forEach(function (s) { s.classList.remove("is-focused"); });
      var el = vis[focusIdx];
      el.classList.add("is-focused");
      list.setAttribute("aria-activedescendant", el.id);
      el.scrollIntoView({ block: "nearest", inline: "nearest", behavior: reduceMotion ? "auto" : "smooth" });
    }

    skins.forEach(function (s) {
      s.addEventListener("click", function () {
        pick(s);
        focusIdx = visibleSkins().indexOf(s);
      });
    });

    list.addEventListener("focus", function () {
      var i = visibleSkins().indexOf(state.skin);
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

    els.prev.hidden = false;
    els.next.hidden = false;
    els.prev.addEventListener("click", function () { step(-1); });
    els.next.addEventListener("click", function () { step(1); });

    // Swipe left / right on the stage (touch).
    var touchX = null;
    stage.addEventListener("touchstart", function (e) {
      touchX = e.touches.length === 1 ? e.touches[0].clientX : null;
    }, { passive: true });
    stage.addEventListener("touchend", function (e) {
      if (touchX === null) return;
      var dx = e.changedTouches[0].clientX - touchX;
      touchX = null;
      if (Math.abs(dx) > 40) step(dx < 0 ? 1 : -1);
    }, { passive: true });

    tablist($(".views", gallery), function (tab) {
      state.view = tab.dataset.view;
      render();
    });

    // Category filter chips
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
        var vis = visibleSkins();
        if (state.skin.hidden && vis[0]) pick(vis[0]);
        else render();
        list.scrollTop = 0;
        list.scrollLeft = 0;
      });
    });

    // Deep link: index.html?skin=xp#skins
    var want = new URLSearchParams(window.location.search).get("skin");
    var match = want && skins.filter(function (s) { return s.dataset.skin === want; })[0];
    if (match) pick(match, true);
    else render(true);
  }

  // ------------------------------------------------------------------
  // Copy buttons on the build console
  // ------------------------------------------------------------------
  $$("[data-copy]").forEach(function (btn) {
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
  // Scroll-spy: mark the nav link of the section being read
  // ------------------------------------------------------------------
  var navLinks = $$('.site-nav ul a[href^="#"]');
  if (navLinks.length && "IntersectionObserver" in window) {
    var byId = {};
    navLinks.forEach(function (a) { byId[a.getAttribute("href").slice(1)] = a; });
    var current = null;
    var spy = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (!en.isIntersecting) return;
        var link = byId[en.target.id];
        if (link === current) return;
        if (current) current.removeAttribute("aria-current");
        current = link;
        if (link) link.setAttribute("aria-current", "location");
      });
    }, { rootMargin: "-45% 0px -50% 0px" });
    Object.keys(byId).forEach(function (id) {
      var sec = document.getElementById(id);
      if (sec) spy.observe(sec);
    });
    // Clear the mark while the hero (no nav entry) is in view.
    var hero = $(".hero");
    if (hero) {
      new IntersectionObserver(function (entries) {
        if (entries[0].isIntersecting && current) {
          current.removeAttribute("aria-current");
          current = null;
        }
      }, { rootMargin: "-45% 0px -50% 0px" }).observe(hero);
    }
  }

  // ------------------------------------------------------------------
  // Reveal-on-scroll (skipped entirely for reduced motion)
  // ------------------------------------------------------------------
  if (!reduceMotion && "IntersectionObserver" in window) {
    var groups = [
      ".section__head", ".contract__steps .step", ".contract__code", ".pillars", ".platforms",
      ".gallery", ".numbers", ".tile", ".arch", ".psp__shots", ".psp__copy", ".feature",
      ".entries", ".blueprint", ".console", ".docs-links", ".cta-band__inner"
    ];
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (en.isIntersecting) {
          en.target.classList.add("is-in");
          io.unobserve(en.target);
        }
      });
    }, { rootMargin: "0px 0px -8% 0px", threshold: 0.05 });
    groups.forEach(function (sel) {
      $$(sel).forEach(function (el, i) {
        // Only animate things below the fold, so nothing visible "pops".
        if (el.getBoundingClientRect().top > window.innerHeight) {
          // Stagger siblings in the same grid a little.
          if (/step|tile|feature|blueprint/.test(sel)) {
            el.style.setProperty("--reveal-delay", (i % 4) * 0.07 + "s");
          }
          el.classList.add("reveal");
          io.observe(el);
        }
      });
    });
  }

  // Pause the hero video when offscreen or when the user prefers reduced motion.
  var video = $(".screen__video");
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
